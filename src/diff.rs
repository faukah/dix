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
use log::warn;
use size::Size;
use unicode_width::UnicodeWidthStr as _;
use yansi::{
  Paint as _,
  Painted,
};

use crate::{
  store, version::{
    VersionComponent, VersionComponentIter, VersionPart, VersionSeparator
  }, StorePath, Version
};

#[derive(Debug, Default)]
struct Diff<T> {
  old: T,
  new: T,
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

// computes the levensthein distance between two strings using
// dynamic programming
fn levenshtein<T: Eq>(from: &[T], to: &[T]) -> usize {
  let (height, width) = (from.len(), to.len());
  let mut old = (0..=width).collect::<Vec<_>>();
  let mut new = vec![0; width + 1];
  for i in 1..=height {
    new[0] = i;
    for j in 1..=width {
      new[j] = min(
        old[j] + 1,
        min(
              new[j - 1] + 1,
                old[j - 1] + usize::from(from[i - 1] == to[j - 1])
            ));
    }
    swap(&mut old, &mut new);
  }
  new[width]
}
/// Takes two lists of versions and tries to match
/// them by first computing the edit distance for all pairs and using
/// the ordering defined on versions as tiebreaker
fn match_version_lists<'a>(
  from: &'a [Version],
  to: &'a [Version],
) -> Vec<EitherOrBoth<&'a Version>> {
  // we store all remaining versions we have not matched yet, keeping
  // the indices to preserve duplicates
  let mut to_remaining = (0..to.len()).collect::<HashSet<usize>>();
  let mut distances: Vec<Vec<usize>> = vec![vec![0; to.len()]; from.len()];
  // TODO: maybe just use a double loop and be done with it
  // compute the complete distance matrix where distance[i][j] := edit distance from from[i]
  for ((i, vfrom), (j, vto)) in itertools::iproduct!(from.iter().enumerate(), to.iter().enumerate()) {
    let components_from:Vec<VersionComponent> = VersionComponentIter::new(vfrom).filter_map(Result::ok).collect();
    let components_to: Vec<VersionComponent> = VersionComponentIter::new(vto).filter_map(Result::ok).collect();
    distances[i][j] = levenshtein(&components_from, &components_to);
  }

  let mut from_remaining = Vec::new();

  let mut pairings = Vec::new();
  for i in 0..from.len() {
    let jmin = distances[i]
      .iter()
      .enumerate()
      .filter(|&(j, _)| to_remaining.contains(&j))
      .min_set_by_key(|&(_, &dist)| dist)
      .into_iter()
      .max_by(|&(left, _), &(right, _)| to[left].cmp(&to[right]))
      .map(|(j, _)| j);

    match jmin {
      Some(j) => {
        pairings.push(EitherOrBoth::Both(&from[i], &to[j]));
        to_remaining.remove(&j);
      },
      None => from_remaining.push(&from[i]),
    }
  }
  let mut to_remaining = to_remaining
    .into_iter()
    .map(|j| &to[j])
    .collect::<Vec<&Version>>();
  from_remaining.sort_unstable();
  to_remaining.sort_unstable();
  pairings.extend(Itertools::zip_longest(
    from_remaining.into_iter(),
    to_remaining,
  ));

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

#[expect(clippy::cognitive_complexity, clippy::too_many_lines)]
fn write_packages_diffln(
  writer: &mut impl fmt::Write,
  paths_old: impl Iterator<Item = StorePath>,
  paths_new: impl Iterator<Item = StorePath>,
  system_paths_old: impl Iterator<Item = StorePath>,
  system_paths_new: impl Iterator<Item = StorePath>,
) -> Result<usize, fmt::Error> {
  let mut paths = HashMap::<String, Diff<Vec<Version>>>::new();

  // Collect the names of old and new paths.
  let system_derivations_old: HashSet<String> = system_paths_old
    .filter_map(|path| {
      match path.parse_name_and_version() {
        Ok((name, _)) => Some(name.into()),
        Err(error) => {
          log::warn!("error parsing old system path name and version: {error}");
          None
        },
      }
    })
    .collect();

  let system_derivations_new: HashSet<String> = system_paths_new
    .filter_map(|path| {
      match path.parse_name_and_version() {
        Ok((name, _)) => Some(name.into()),
        Err(error) => {
          log::warn!("error parsing new system path name and version: {error}");
          None
        },
      }
    })
    .collect();

  for path in paths_old {
    match path.parse_name_and_version() {
      Ok((name, version)) => {
        log::trace!("parsed name: {name}");
        log::trace!("parsed version: {version:?}");

        paths
          .entry(name.into())
          .or_default()
          .old
          .push(version.unwrap_or_else(|| Version::from("<none>".to_owned())));
      },

      Err(error) => {
        log::warn!("error parsing old path name and version: {error}");
      },
    }
  }

  for path in paths_new {
    match path.parse_name_and_version() {
      Ok((name, version)) => {
        log::trace!("parsed name: {name}");
        log::trace!("parsed version: {version:?}");

        paths
          .entry(name.into())
          .or_default()
          .new
          .push(version.unwrap_or_else(|| Version::from("<none>".to_owned())));
      },

      Err(error) => {
        log::warn!("error parsing new path name and version: {error}");
      },
    }
  }
  let mut diffs = paths
    .into_iter()
    .filter_map(|(name, mut versions)| {
      deduplicate_versions(&mut versions.old);
      deduplicate_versions(&mut versions.new);

      let old_copy = versions.old.clone();
      let new_copy = versions.new.clone();

      versions.old.retain(|ver| !new_copy.contains(ver));
      versions.new.retain(|ver| !old_copy.contains(ver));

      let status = match (versions.old.len(), versions.new.len()) {
        (0, 0) => return None,
        (0, _) => DiffStatus::Added,
        (_, 0) => DiffStatus::Removed,
        _ => {
          let mut saw_upgrade = false;
          let mut saw_downgrade = false;

          for diff in match_version_lists(&versions.old, &versions.new)
          {
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
            _ => return None,
          })
        },
      };

      let selection = DerivationSelectionStatus::from_names(
        &name,
        &system_derivations_old,
        &system_derivations_new,
      );

      Some((name, versions, status, selection))
    })
    .collect::<Vec<_>>();

  diffs.sort_by(
    |&(ref a_name, _, a_status, _), &(ref b_name, _, b_status, _)| {
      a_status.cmp(&b_status).then_with(|| a_name.cmp(b_name))
    },
  );

  #[expect(clippy::pattern_type_mismatch)]
  let name_width = diffs
    .iter()
    .map(|(name, ..)| name.width())
    .max()
    .unwrap_or(0);

  let mut last_status = None::<DiffStatus>;

  for &(ref name, ref versions, status, selection) in &diffs {
    if last_status.is_none_or(|last_status| {
      // Using the Ord implementation instead of Eq on purpose.
      // Eq returns false for DiffStatus::Changed(X) == DiffStatus::Changed(Y).
      last_status.cmp(&status) != cmp::Ordering::Equal
    }) {
      writeln!(
        writer,
        "{nl}{status}",
        nl = if last_status.is_some() { "\n" } else { "" },
        status = match status {
          DiffStatus::Changed(_) => "CHANGED",
          DiffStatus::Added => "ADDED",
          DiffStatus::Removed => "REMOVED",
        }
        .bold(),
      )?;

      last_status = Some(status);
    }

    let status = status.char();
    let selection = selection.char();
    let name = name.paint(selection.style);

    write!(writer, "[{status}{selection}] {name:<name_width$}")?;

    let mut oldacc = String::new();
    let mut oldwrote = false;
    let mut newacc = String::new();
    let mut newwrote = false;

    for diff in match_version_lists(&versions.old, &versions.new)
    {
      match diff {
        EitherOrBoth::Left(old_version) => {
          if oldwrote {
            write!(oldacc, ", ")?;
          } else {
            write!(oldacc, " ")?;
            oldwrote = true;
          }

          for old_comp in old_version {
            match old_comp {
              VersionPart(old_comp) => {
                write!(oldacc, "{old}", old = old_comp.red())?;
              },
              VersionSeparator(ignored) => write!(oldacc, "{ignored}")?,
            }
          }
        },

        EitherOrBoth::Right(new_version) => {
          if newwrote {
            write!(newacc, ", ")?;
          } else {
            write!(newacc, " ")?;
            newwrote = true;
          }

          for new_comp in new_version {
            match new_comp {
              VersionPart(new_comp) => {
                write!(newacc, "{new}", new = new_comp.green())?;
              },
              VersionSeparator(ignored) => write!(newacc, "{ignored}")?,
            }
          }
        },

        EitherOrBoth::Both(old_version, new_version) => {
          if old_version == new_version {
            continue;
          }

          if oldwrote {
            write!(oldacc, ", ")?;
          } else {
            write!(oldacc, " ")?;
            oldwrote = true;
          }
          if newwrote {
            write!(newacc, ", ")?;
          } else {
            write!(newacc, " ")?;
            newwrote = true;
          }

          let mut old_version: Vec<_> = old_version.into_iter().collect();
          let mut new_version: Vec<_> = new_version.into_iter().collect();

          let mut common_suffix = Vec::new();

          while old_version.last() == new_version.last() {
            old_version.pop();
            common_suffix.push(new_version.pop());
          }

          for diff in Itertools::zip_longest(
            old_version.into_iter(),
            new_version.into_iter(),
          ) {
            match diff {
              EitherOrBoth::Left(old_comp) => {
                match old_comp {
                  VersionPart(old_comp) => {
                    write!(oldacc, "{old}", old = old_comp.red())?;
                  },
                  VersionSeparator(ignored) => {
                    write!(oldacc, "{ignored}")?;
                  },
                }
              },

              EitherOrBoth::Right(new_comp) => {
                match new_comp {
                  VersionPart(new_comp) => {
                    write!(newacc, "{new}", new = new_comp.green())?;
                  },
                  VersionSeparator(ignored) => {
                    write!(newacc, "{ignored}")?;
                  },
                }
              },

              EitherOrBoth::Both(old_comp, new_comp) => {
                match (old_comp, new_comp) {
                  (VersionPart(old_comp), VersionPart(new_comp)) => {
                    let mut difference_started = false;
                    let is_hash = is_hash(&old_comp);

                    for char in diff::chars(*old_comp, *new_comp) {
                      match char {
                        diff::Result::Both(old_part, new_part) => {
                          if difference_started {
                            difference_started = true;
                            write!(oldacc, "{old}", old = old_part.red())?;
                            write!(newacc, "{new}", new = new_part.green())?;
                          } else {
                            write!(oldacc, "{old}", old = old_part.yellow())?;
                            write!(newacc, "{new}", new = new_part.yellow())?;
                          }
                        },
                        diff::Result::Left(old_part) => {
                          difference_started = is_hash;
                          write!(oldacc, "{old}", old = old_part.red())?;
                        },
                        diff::Result::Right(new_part) => {
                          difference_started = is_hash;
                          write!(newacc, "{new}", new = new_part.green())?;
                        },
                      }
                    }
                  },
                  (old_comp, new_comp) => {
                    match old_comp {
                      VersionPart(old_comp) => {
                        write!(oldacc, "{old}", old = old_comp.red())?;
                      },
                      VersionSeparator(old_comp) => {
                        write!(oldacc, "{old_comp}")?;
                      },
                    }

                    match new_comp {
                      VersionPart(new_comp) => {
                        write!(newacc, "{new}", new = new_comp.green())?;
                      },
                      VersionSeparator(new_comp) => {
                        write!(newacc, "{new_comp}")?;
                      },
                    }
                  },
                }
              },
            }
          }

          for comp in common_suffix.into_iter().rev().flatten() {
            match comp {
              VersionPart(comp) => {
                write!(oldacc, "{old}", old = comp.yellow())?;
                write!(newacc, "{new}", new = comp.yellow())?;
              },
              VersionSeparator(ignored) => {
                write!(oldacc, "{ignored}")?;
                write!(newacc, "{ignored}")?;
              },
            }
          }
        },
      }
    }

    write!(
      writer,
      "{oldacc}{arrow}{newacc}",
      arrow = if !oldacc.is_empty() && !newacc.is_empty() {
        " ->"
      } else {
        ""
      }
    )?;

    writeln!(writer)?;
  }

  Ok(diffs.len())
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
    use crate::{diff::levenshtein, version::{VersionComponent, VersionComponentIter}};

  #[test]
  fn basic_component_edit_dist() {
    let from: Vec<VersionComponent> = VersionComponentIter::new("foo-123.0-man-pages").filter_map(Result::ok).collect();
    let to: Vec<VersionComponent> = VersionComponentIter::new("foo-123.4.12-man-pages").filter_map(Result::ok).collect();    
    let dist = levenshtein(&from, &to);
    assert_eq!(dist, 2);
  }
}
