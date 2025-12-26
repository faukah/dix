use std::{
  cmp::{
    self,
    min,
  },
  collections::{
    HashMap,
    HashSet,
  },
  fmt::{
    self,
    Write as _,
  },
  fs,
  mem::swap,
  path::{
    Path,
    PathBuf,
  },
  thread,
};

use anyhow::{
  Context as _,
  Error,
  Result,
};
use itertools::{
  EitherOrBoth,
  Itertools,
};
use pathfinding::{
  kuhn_munkres,
  matrix::Matrix,
};
use size::Size;
use unicode_width::UnicodeWidthStr as _;
use yansi::{
  Paint as _,
  Painted,
};

use crate::{
  StorePath,
  Version,
  store,
  version::{
    VersionComponent,
    VersionPiece,
  },
};

#[derive(Debug, Default, Eq, PartialEq)]
struct Diff<T> {
  old: T,
  new: T,
}

#[derive(Debug, Eq, PartialEq)]
struct DetailedDiff {
  name:      String,
  diff:      Diff<Vec<Version>>,
  status:    DiffStatus,
  selection: DerivationSelectionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
  UpgradeDowngrade,
  Upgraded,
  Downgraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffStatus {
  Changed(Change),
  Added,
  Removed,
}

impl DiffStatus {
  fn char(self) -> Painted<&'static char> {
    match self {
      Self::Changed(Change::UpgradeDowngrade) => 'C'.yellow().bold(),
      Self::Changed(Change::Upgraded) => 'U'.bright_cyan().bold(),
      Self::Changed(Change::Downgraded) => 'D'.magenta().bold(),
      Self::Added => 'A'.green().bold(),
      Self::Removed => 'R'.red().bold(),
    }
  }
}

impl PartialOrd for DiffStatus {
  fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl cmp::Ord for DiffStatus {
  fn cmp(&self, other: &Self) -> cmp::Ordering {
    use DiffStatus::{
      Added,
      Changed,
      Removed,
    };
    #[expect(clippy::match_same_arms)]
    match (*self, *other) {
      (Changed(_), Changed(_)) => cmp::Ordering::Equal,
      (Added, Added) => cmp::Ordering::Equal,
      (Removed, Removed) => cmp::Ordering::Equal,

      (Changed(_), _) => cmp::Ordering::Less,
      (_, Changed(_)) => cmp::Ordering::Greater,

      (Added, Removed) => cmp::Ordering::Less,
      (Removed, Added) => cmp::Ordering::Greater,
    }
  }
}

/// Documents if the derivation is a system package and if
/// it was added / removed as such.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DerivationSelectionStatus {
  /// The derivation is a system package, status unchanged.
  Selected,
  /// The derivation was not a system package before but is now.
  NewlySelected,
  /// The derivation is and was a dependency.
  Unselected,
  /// The derivation was a system package before but is not anymore.
  NewlyUnselected,
}

impl DerivationSelectionStatus {
  fn from_names(
    name: &str,
    old: &HashSet<String>,
    new: &HashSet<String>,
  ) -> Self {
    match (old.contains(name), new.contains(name)) {
      (true, true) => Self::Selected,
      (true, false) => Self::NewlyUnselected,
      (false, true) => Self::NewlySelected,
      (false, false) => Self::Unselected,
    }
  }

  fn char(self) -> Painted<&'static char> {
    match self {
      Self::Selected => '*'.bold(),
      Self::NewlySelected => '+'.bold(),
      Self::Unselected => Painted::new(&'.'),
      Self::NewlyUnselected => Painted::new(&'-'),
    }
  }
}

/// Writes the diff header (<<< out, >>>in) and package diff.
///
/// # Returns
///
/// Will return the amount of package diffs written. Even when zero,
/// the header will be written.
#[expect(clippy::missing_errors_doc)]
pub fn write_paths_diffln(
  writer: &mut impl fmt::Write,
  path_old: &Path,
  path_new: &Path,
) -> Result<usize> {
  let connection = store::connect()?;

  let paths_old = connection
    .query_dependents(path_old)
    .with_context(|| {
      format!(
        "failed to query dependencies of path '{path}'",
        path = path_old.display()
      )
    })?
    .map(|(_, path)| path);

  let paths_new = connection
    .query_dependents(path_new)
    .with_context(|| {
      format!(
        "failed to query dependencies of path '{path}'",
        path = path_new.display()
      )
    })?
    .map(|(_, path)| path);

  let system_derivations_old = connection
    .query_system_derivations(path_old)
    .with_context(|| {
      format!(
        "failed to query system derivations of path '{path}",
        path = path_old.display()
      )
    })?
    .map(|(_, path)| path);

  let system_derivations_new = connection
    .query_system_derivations(path_new)
    .with_context(|| {
      format!(
        "failed to query system derivations of path '{path}",
        path = path_old.display()
      )
    })?
    .map(|(_, path)| path);

  writeln!(
    writer,
    "{arrows} {old}",
    arrows = "<<<".bold(),
    old = path_old.display(),
  )?;
  writeln!(
    writer,
    "{arrows} {new}",
    arrows = ">>>".bold(),
    new = fs::canonicalize(path_new)
      .unwrap_or_else(|_| path_new.to_path_buf())
      .display(),
  )?;

  writeln!(writer)?;

  Ok(write_packages_diffln(
    writer,
    paths_old,
    paths_new,
    system_derivations_old,
    system_derivations_new,
  )?)
}

// Computes the levensthein distance between two strings using
// dynamic programming.
fn levenshtein<T: Eq>(from: &[T], to: &[T]) -> usize {
  let (from_len, to_len) = (from.len(), to.len());
  if from_len == 0 {
    return to_len;
  }
  if to_len == 0 {
    return from_len;
  }

  let mut prev_row: Vec<_> = (0..=to_len).collect();
  let mut curr_row = vec![0; to_len + 1];

  for i in 1..=from_len {
    curr_row[0] = i;
    for j in 1..=to_len {
      let subcost = usize::from(from[i - 1] != to[j - 1]);
      curr_row[j] = min(
        min(curr_row[j - 1] + 1, prev_row[j] + 1),
        prev_row[j - 1] + subcost,
      );
    }
    swap(&mut prev_row, &mut curr_row);
  }
  prev_row[to_len]
}

/// Takes two lists of versions and tries to match
/// them by first computing the edit distance for all pairs and using
/// the ordering defined on versions as tiebreaker.
/// TODO: we might want to implement the hungarian algorithm ourselves
fn match_version_lists<'a>(
  mut from: &'a [Version],
  mut to: &'a [Version],
) -> Vec<EitherOrBoth<&'a Version>> {
  // the hungarian algorithm for finding
  // matchings requires #rows <= #columns
  // Since the edit distance is symmetric,
  // we can just swap the values
  let swapped = if from.len() > to.len() {
    (to, from) = (from, to);
    true
  } else {
    false
  };

  let mut distances = Matrix::new(from.len(), to.len(), 0_i32);

  // Compute the complete distance matrix where distance[i][j] := edit distance
  // from from[i] to to[j].
  for (i, from_version) in from.iter().enumerate() {
    for (j, to_version) in to.iter().enumerate() {
      let components_from: Vec<VersionComponent> = from_version
        .into_iter()
        .filter_map(VersionPiece::component)
        .collect();

      let components_to: Vec<VersionComponent> = to_version
        .into_iter()
        .filter_map(VersionPiece::component)
        .collect();

      distances[(i, j)] =
        i32::try_from(levenshtein(&components_from, &components_to))
          .expect("distance must fit in i32");
    }
  }
  let (_cost, matchings) =
    kuhn_munkres::kuhn_munkres_min::<i32, Matrix<i32>>(&distances);

  let mut remaining = (0..to.len()).collect::<HashSet<usize>>();
  let mut pairings = Vec::<EitherOrBoth<&Version>>::new();
  for (i, j) in matchings.into_iter().enumerate() {
    pairings.push(EitherOrBoth::Both(&from[i], &to[j]));
    remaining.remove(&j);
  }
  // some vertices in two might still not be matched so add those as well at the
  // end
  let mut remaining = remaining.iter().map(|&j| &to[j]).collect::<Vec<_>>();
  remaining.sort_unstable();
  pairings.extend(remaining.into_iter().map(EitherOrBoth::Right));

  // swap everything back
  if swapped {
    pairings = pairings.into_iter().map(EitherOrBoth::flip).collect();
  }

  pairings
}

/// Takes a list of versions which may contain duplicates and deduplicates it by
/// replacing multiple occurrences of an element with the same element plus the
/// amount it occurs.
///
/// # Example
///
/// ```rs
/// let mut versions = vec!["2.3", "1.0", "2.3", "4.8", "2.3", "1.0"];
///
/// deduplicate_versions(&mut versions);
/// assert_eq!(*versions, &["1.0 ×2", "2.3 ×3", "4.8"]);
/// ```
fn deduplicate_versions(versions: &mut Vec<Version>) {
  versions.sort_unstable();

  let mut deduplicated = Vec::new();

  // Push a version onto the final vec. If it occurs more than once,
  // we add a ×{count} to signify the amount of times it occurs.
  let mut deduplicated_push = |mut version: Version, count: usize| {
    if count > 1 {
      write!(version, " ×{count}").unwrap();
    }
    deduplicated.push(version);
  };

  let mut last_version = None::<(Version, usize)>;
  for version in versions.iter() {
    #[expect(clippy::mixed_read_write_in_expression)]
    let Some((last_version_value, count)) = last_version.take() else {
      last_version = Some((version.clone(), 1));
      continue;
    };

    // If the last version matches the current version, we increase the count by
    // one. Otherwise, we push the last version to the result.
    if last_version_value == *version {
      last_version = Some((last_version_value, count + 1));
    } else {
      deduplicated_push(last_version_value, count);
      last_version = Some((version.clone(), 1));
    }
  }

  // Push the final element, if it exists.
  if let Some((version, count)) = last_version.take() {
    deduplicated_push(version, count);
  }

  *versions = deduplicated;
}

/// Entry point for writing package differences.
pub fn write_packages_diffln(
  writer: &mut impl fmt::Write,
  paths_old: impl Iterator<Item = StorePath>,
  paths_new: impl Iterator<Item = StorePath>,
  system_paths_old: impl Iterator<Item = StorePath>,
  system_paths_new: impl Iterator<Item = StorePath>,
) -> Result<usize, fmt::Error> {
  let paths_map = collect_path_versions(paths_old, paths_new);
  let sys_old_set = collect_system_names(system_paths_old, "old");
  let sys_new_set = collect_system_names(system_paths_new, "new");

  let mut diffs = generate_diffs_from_paths(paths_map, &Diff {
    old: sys_old_set,
    new: sys_new_set,
  });

  // We want to sort the diffs by their diff status, e.g.:
  // CHANGED
  // ...
  // ...
  //
  // ADDDED
  // ...
  // ...
  //
  // REMOVED
  // ...
  // ...
  // The diffs themselves get sorted by name inside of their sections.
  #[expect(clippy::min_ident_chars)]
  diffs
    .sort_by(|a, b| a.status.cmp(&b.status).then_with(|| a.name.cmp(&b.name)));

  render_diffs(writer, &diffs)
}

// --- Data Collection Helpers ---

fn collect_system_names(
  paths: impl Iterator<Item = StorePath>,
  context: &str,
) -> HashSet<String> {
  paths
    .filter_map(|path| {
      match path.parse_name_and_version() {
        Ok((name, _)) => Some(name.into()),
        Err(error) => {
          log::warn!("error parsing {context} system path name: {error}");
          None
        },
      }
    })
    .collect()
}

fn collect_path_versions(
  old: impl Iterator<Item = StorePath>,
  new: impl Iterator<Item = StorePath>,
) -> HashMap<String, Diff<Vec<Version>>> {
  let mut paths = HashMap::<String, Diff<Vec<Version>>>::new();

  let mut add_ver = |path: StorePath, is_old: bool| {
    match path.parse_name_and_version() {
      Ok((name, version)) => {
        let entry = paths.entry(name.into()).or_default();
        let list = if is_old {
          &mut entry.old
        } else {
          &mut entry.new
        };
        list
          .push(version.unwrap_or_else(|| Version::from("<none>".to_owned())));
      },
      Err(e) => log::warn!("error parsing path: {e}"),
    }
  };

  for p in old {
    add_ver(p, true);
  }
  for p in new {
    add_ver(p, false);
  }

  paths
}

fn render_diffs(
  writer: &mut impl fmt::Write,
  diffs: &[DetailedDiff],
) -> Result<usize, fmt::Error> {
  let name_width = diffs.iter().map(|d| d.name.width()).max().unwrap_or(0) + 1;

  let mut last_status = None::<DiffStatus>;

  for d in diffs {
    // Print Section Header (CHANGED, ADDED, REMOVED)
    if last_status
      .map_or_else(|| true, |ls| ls.cmp(&d.status) != cmp::Ordering::Equal)
    {
      if last_status.is_some() {
        writeln!(writer)?;
      }
      let header = match d.status {
        DiffStatus::Changed(_) => "CHANGED",
        DiffStatus::Added => "ADDED",
        DiffStatus::Removed => "REMOVED",
      }
      .bold();
      writeln!(writer, "{header}")?;
      last_status = Some(d.status);
    }

    // Print Package Name
    let status_char = d.status.char();
    let sel_char = d.selection.char();
    let name_painted = d.name.paint(sel_char.style);
    write!(
      writer,
      "[{status_char}{sel_char}] {name_painted:<name_width$}"
    )?;

    // Print Version Differences
    let (old_str, new_str) = fmt_version_diffs(&d.diff.old, &d.diff.new)?;

    let arrow = if !old_str.is_empty() && !new_str.is_empty() {
      " -> "
    } else {
      ""
    };
    writeln!(writer, "{old_str}{arrow}{new_str}")?;
  }

  Ok(diffs.len())
}

/// Generates the colored strings for the old and new versions.
fn fmt_version_diffs(
  old_versions: &[Version],
  new_versions: &[Version],
) -> Result<(String, String), fmt::Error> {
  let mut old_acc = String::new();
  let mut new_acc = String::new();
  let mut old_wrote = false;
  let mut new_wrote = false;

  let append_sep = |acc: &mut String, wrote: &mut bool| {
    if *wrote {
      write!(acc, ", ")
    } else {
      *wrote = true;
      Ok(())
    }
  };

  for diff in match_version_lists(old_versions, new_versions) {
    match diff {
      EitherOrBoth::Left(old) => {
        append_sep(&mut old_acc, &mut old_wrote)?;
        for comp in old {
          write_ver_piece(&mut old_acc, &comp, |c| c.red())?;
        }
      },
      EitherOrBoth::Right(new) => {
        append_sep(&mut new_acc, &mut new_wrote)?;
        for comp in new {
          write_ver_piece(&mut new_acc, &comp, |c| c.green())?;
        }
      },
      EitherOrBoth::Both(old, new) => {
        if old == new {
          continue;
        }

        append_sep(&mut old_acc, &mut old_wrote)?;
        append_sep(&mut new_acc, &mut new_wrote)?;

        fmt_version_diff(&mut old_acc, &mut new_acc, old, new)?;
      },
    }
  }
  Ok((old_acc, new_acc))
}

fn write_ver_piece(
  buf: &mut String,
  piece: &VersionPiece,
  style: impl Fn(Painted<&str>) -> Painted<&str>,
) -> fmt::Result {
  match piece {
    VersionPiece::Component(c) => write!(buf, "{}", style(Painted::new(c))),
    VersionPiece::Separator(s) => write!(buf, "{s}"),
  }
}

/// Handles the logic of comparing two specific versions:
/// 1. Finds common prefixes and suffixes, which are colored yellow.
/// 2. Compares the remaining middle parts, with removals in red and additions
///    in green.
fn fmt_version_diff(
  old_acc: &mut String,
  new_acc: &mut String,
  old_ver: &Version,
  new_ver: &Version,
) -> fmt::Result {
  let old_parts: Vec<_> = old_ver.into_iter().collect();
  let new_parts: Vec<_> = new_ver.into_iter().collect();

  // Find the lengths of the common prefix and suffix to isolate the differing
  // middle parts.
  let prefix_len = old_parts
    .iter()
    .zip(&new_parts)
    .take_while(|(a, b)| a == b)
    .count();

  let suffix_len = old_parts[prefix_len..]
    .iter()
    .rev()
    .zip(new_parts[prefix_len..].iter().rev())
    .take_while(|(a, b)| a == b)
    .count();

  // Get slices for the three sections: prefix, diff, and suffix.
  let prefix = &old_parts[..prefix_len];
  let old_diff = &old_parts[prefix_len..old_parts.len() - suffix_len];
  let new_diff = &new_parts[prefix_len..new_parts.len() - suffix_len];
  let suffix = &old_parts[old_parts.len() - suffix_len..];

  // Write common prefix (yellow).
  for piece in prefix {
    write_ver_piece(old_acc, piece, |c| c.yellow())?;
    write_ver_piece(new_acc, piece, |c| c.yellow())?;
  }

  // Write differing middle parts (red/green).
  for pair in Itertools::zip_longest(old_diff.into_iter(), new_diff.into_iter())
  {
    match pair {
      EitherOrBoth::Left(old) => write_ver_piece(old_acc, old, |c| c.red())?,
      EitherOrBoth::Right(new) => write_ver_piece(new_acc, new, |c| c.green())?,
      EitherOrBoth::Both(old, new) => {
        fmt_version_piece_pair(old_acc, new_acc, old, new)?;
      },
    }
  }

  // Write common suffix (yellow).
  for piece in suffix {
    write_ver_piece(old_acc, piece, |c| c.yellow())?;
    write_ver_piece(new_acc, piece, |c| c.yellow())?;
  }

  Ok(())
}

/// Compares and formats two `VersionPieces`.
fn fmt_version_piece_pair(
  old_acc: &mut String,
  new_acc: &mut String,
  old_piece: &VersionPiece,
  new_piece: &VersionPiece,
) -> fmt::Result {
  match (old_piece, new_piece) {
    // If both the old and the new component are of type
    // `VersionPiece::Component`, we want to do character-level diffing.
    (VersionPiece::Component(old_c), VersionPiece::Component(new_c)) => {
      let char_diffs: Vec<_> = diff::chars(old_c, new_c);

      let hash_mode = is_hash(old_c);
      let mut diff_active = false;

      for res in char_diffs {
        match res {
          diff::Result::Both(l, r) => {
            if diff_active {
              write!(old_acc, "{}", l.red())?;
              write!(new_acc, "{}", r.green())?;
            } else {
              write!(old_acc, "{}", l.yellow())?;
              write!(new_acc, "{}", r.yellow())?;
            }
          },
          diff::Result::Left(l) => {
            diff_active = hash_mode;
            write!(old_acc, "{}", l.red())?;
          },
          diff::Result::Right(r) => {
            diff_active = hash_mode;
            write!(new_acc, "{}", r.green())?;
          },
        }
      }
    },
    // If one is a separator and the other isn't, or both separators differ
    (o, n) => {
      write_ver_piece(old_acc, o, |c| c.red())?;
      write_ver_piece(new_acc, n, |c| c.green())?;
    },
  }
  Ok(())
}

/// Spawns a task to compute the data required by [`write_size_diffln`].
#[must_use]
pub fn spawn_size_diff(
  path_old: PathBuf,
  path_new: PathBuf,
) -> thread::JoinHandle<Result<(Size, Size)>> {
  log::debug!("calculating closure sizes in background");

  thread::spawn(move || {
    let connection = store::connect()?;

    Ok::<_, Error>((
      connection.query_closure_size(&path_old)?,
      connection.query_closure_size(&path_new)?,
    ))
  })
}

/// Writes the size difference between two numbers to `writer`.
///
/// # Returns
///
/// Will return nothing when successful.
///
/// # Errors
///
/// Returns `Err` when writing to `writer` fails.
pub fn write_size_diffln(
  writer: &mut impl fmt::Write,
  size_old: Size,
  size_new: Size,
) -> fmt::Result {
  let size_diff = size_new - size_old;

  writeln!(
    writer,
    "{header}: {size_old} -> {size_new}",
    header = "SIZE".bold(),
    size_old = size_old.red(),
    size_new = size_new.green(),
  )?;

  writeln!(
    writer,
    "{header}: {size_diff}",
    header = "DIFF".bold(),
    size_diff = if size_diff.bytes() > 0 {
      size_diff.green()
    } else {
      size_diff.red()
    },
  )
}

fn generate_diffs_from_paths(
  paths: HashMap<String, Diff<Vec<Version>>>,
  system_paths: &Diff<HashSet<String>>,
) -> Vec<DetailedDiff> {
  paths
    .into_iter()
    .filter_map(|(name, mut versions)| {
      deduplicate_versions(&mut versions.old);
      deduplicate_versions(&mut versions.new);

      // 1. Convert to HashSets for O(1) lookups
      let old_set: HashSet<_> = versions.old.iter().cloned().collect();
      let new_set: HashSet<_> = versions.new.iter().cloned().collect();

      let common_versions_count = old_set.intersection(&new_set).count();

      versions.old.retain(|ver| !new_set.contains(ver));
      versions.new.retain(|ver| !old_set.contains(ver));

      let status = match (versions.old.len(), versions.new.len()) {
        (0, 0) => return None,
        (0, _) => DiffStatus::Added,
        (_, 0) => DiffStatus::Removed,
        _ => {
          let mut saw_upgrade = false;
          let mut saw_downgrade = false;

          for diff in match_version_lists(&versions.old, &versions.new) {
            match diff {
              EitherOrBoth::Left(_) => saw_downgrade = true,
              EitherOrBoth::Right(_) => saw_upgrade = true,

              EitherOrBoth::Both(old, new) => {
                match old.cmp(new) {
                  cmp::Ordering::Less => saw_upgrade = true,
                  cmp::Ordering::Greater => saw_downgrade = true,
                  cmp::Ordering::Equal => {},
                }

                if saw_upgrade && saw_downgrade {
                  break;
                }
              },
            }
          }

          DiffStatus::Changed(match (saw_upgrade, saw_downgrade) {
            (true, true) => Change::UpgradeDowngrade,
            (true, false) => Change::Upgraded,
            (false, true) => Change::Downgraded,
            (false, false) => return None,
          })
        },
      };

      // Show an `<others>` version field if there is at least *one* common
      // version, and exactly one of the two lists of retained versions
      // has a length of at most 1. This helps prevent issues like
      // `https://github.com/faukah/dix/issues/40`,
      // where packages get flagged as removed incorrectly, since the update
      // drops *some* versions of a package, but not all.
      //
      // This code does explicitly *not* catch cases like
      // `["1.2.0", "1.3"] -> ["1.2.0", "1.5"]`, since that should be correctly
      // shown as an upgrade, i.e. `1.3 -> 1.5`.
      // `["1.2.0", "1.3"] -> ["1.2.0"]` would however be caught and result in
      // `<others>`
      if common_versions_count > 0
        && ((versions.old.len() <= 1 && versions.new.len() == 0)
          || (versions.new.len() <= 1 && versions.old.len() == 0))
      {
        println!("AAAAAAAAAA");
        println!("old: {:?} new: {:?}", versions.old, versions.new);
        println!(
          "old: {:?} new: {:?} common: {}",
          versions.old.len(),
          versions.new.len(),
          common_versions_count
        );
        let old_part = versions.old.get(0);
        let new_part = versions.new.get(0);
        if let Some(old) = old_part {
          versions.old = vec![old.clone(), Version("<others>".to_owned())];
        } else {
          versions.old = vec![Version("<others>".to_owned())];
        }
        if let Some(new) = new_part {
          versions.new = vec![new.clone(), Version("<others>".to_owned())];
        } else {
          versions.new = vec![Version("<others>".to_owned())];
        }
      }

      let selection = DerivationSelectionStatus::from_names(
        &name,
        &system_paths.old,
        &system_paths.new,
      );

      let diff = DetailedDiff {
        name,
        diff: versions,
        status,
        selection,
      };

      Some(diff)
    })
    .collect()
}

#[expect(clippy::cast_precision_loss)]
fn dissimilar_score(input: &str) -> f64 {
  input
    .chars()
    .chunk_by(char::is_ascii_digit)
    .into_iter()
    .map(|(_, chunk)| (2.1_f64).powf(chunk.count() as f64))
    .sum::<f64>()
    / input.chars().count() as f64
}

fn is_hash(input: &str) -> bool {
  dissimilar_score(input) < 70.0
}

#[cfg(test)]
mod tests {
  use crate::{
    diff::{
      levenshtein,
      match_version_lists,
    },
    version::{
      Version,
      VersionComponent,
      VersionPiece,
    },
  };

  #[test]
  fn basic_component_edit_dist() {
    let from = Version::from("foo-123.0-man-pages".to_owned());
    let from: Vec<VersionComponent> = from
      .into_iter()
      .filter_map(VersionPiece::component)
      .collect();

    let to = Version::from("foo-123.4.12-man-pages".to_owned());
    let to: Vec<VersionComponent> =
      to.into_iter().filter_map(VersionPiece::component).collect();

    let dist = levenshtein(&from, &to);
    assert_eq!(dist, 2);
  }

  #[test]
  fn levenshtein_distance_tests() {
    assert_eq!(
      levenshtein(
        &"kitten".chars().collect::<Vec<_>>(),
        &"sitting".chars().collect::<Vec<_>>()
      ),
      3
    );

    assert_eq!(
      levenshtein(
        &"".chars().collect::<Vec<_>>(),
        &"hello".chars().collect::<Vec<_>>()
      ),
      5
    );

    assert_eq!(
      levenshtein(
        &"abcd".chars().collect::<Vec<_>>(),
        &"dcba".chars().collect::<Vec<_>>()
      ),
      4
    );

    assert_eq!(
      levenshtein(
        &"12345".chars().collect::<Vec<_>>(),
        &"12345".chars().collect::<Vec<_>>()
      ),
      0
    );

    assert_eq!(
      levenshtein(
        &"distance".chars().collect::<Vec<_>>(),
        &"difference".chars().collect::<Vec<_>>()
      ),
      5
    );
  }

  #[test]
  fn match_version_lists_test() {
    use crate::version::Version;
    let version_list_a = [
      Version("5.116.0".to_owned()),
      Version("5.116.0-bin".to_owned()),
      Version("6.16.0".to_owned()),
    ];
    let version_list_b = [Version("6.17.0".to_owned())];

    let matched = match_version_lists(&version_list_a, &version_list_b);

    for version in matched {
      match version {
        itertools::EitherOrBoth::Both(left, right) => {
          println!("{left} {right}");
        },
        itertools::EitherOrBoth::Left(left) => {
          println!("{left}");
        },
        itertools::EitherOrBoth::Right(right) => {
          println!("{right}");
        },
      }
    }
  }
}

#[test]
fn test_generate_diffs_from_paths() {
  use crate::version::Version;
  let mut paths: HashMap<String, Diff<Vec<Version>>> = HashMap::new();

  let diff_1: Diff<Vec<Version>> = Diff {
    old: vec![Version("1.1.0".to_owned()), Version("1.3".to_owned())],
    new: vec![Version("1.1.0".to_owned()), Version("1.4".to_owned())],
  };
  let system_paths = Diff {
    old: HashSet::<String>::new(),
    new: HashSet::<String>::new(),
  };
  paths.insert("tmp1".to_owned(), diff_1);
  let vec_1 = generate_diffs_from_paths(paths, &system_paths);
  let res_1 = DetailedDiff {
    name:      "tmp1".to_owned(),
    diff:      Diff {
      old: vec![Version("1.3".to_owned())],
      new: vec![Version("1.4".to_owned())],
    },
    status:    DiffStatus::Changed(Change::Upgraded),
    selection: DerivationSelectionStatus::Unselected,
  };
  assert_eq!(vec_1.first().unwrap(), &res_1);

  paths = HashMap::new();

  let diff_2: Diff<Vec<Version>> = Diff {
    old: vec![Version("1.2.0".to_owned()), Version("1.5".to_owned())],
    new: vec![Version("1.2.0".to_owned())],
  };
  paths.insert("tmp".to_owned(), diff_2);
  let vec_2 = generate_diffs_from_paths(paths, &system_paths);
  let res_1 = DetailedDiff {
    name:      "tmp".to_owned(),
    diff:      Diff {
      old: vec![Version("1.5".to_owned()), Version("<others>".to_owned())],
      new: vec![Version("<others>".to_owned())],
    },
    status:    DiffStatus::Removed,
    selection: DerivationSelectionStatus::Unselected,
  };
  assert_eq!(vec_2.first().unwrap(), &res_1);
}
