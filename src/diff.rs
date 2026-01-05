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
  mem::swap,
  path::{
    Path,
    PathBuf,
  },
  thread,
};

use ::std::hash::BuildHasher;
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

#[derive(Debug, Eq, PartialEq)]
pub struct Diff<T = Vec<Version>> {
  pub name:                String,
  pub old:                 T,
  pub new:                 T,
  pub status:              DiffStatus,
  pub selection:           DerivationSelectionStatus,
  pub has_common_versions: bool,
}

impl<T> Default for Diff<T>
where
  T: Default,
{
  fn default() -> Self {
    Self {
      name:                String::default(),
      old:                 T::default(),
      new:                 T::default(),
      status:              DiffStatus::Changed(Change::UpgradeDowngrade),
      selection:           DerivationSelectionStatus::Unselected,
      has_common_versions: false,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
  UpgradeDowngrade,
  Upgraded,
  Downgraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffStatus {
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
    // Define a consistent ordering:
    // Changed comes first, then Added, then Removed
    #[expect(clippy::pattern_type_mismatch, clippy::match_same_arms)]
    match (self, other) {
      // Same variants are equal
      (Self::Changed(_), Self::Changed(_)) => cmp::Ordering::Equal,
      (Self::Added, Self::Added) => cmp::Ordering::Equal,
      (Self::Removed, Self::Removed) => cmp::Ordering::Equal,

      // Changed comes before everything else
      (Self::Changed(_), _) => cmp::Ordering::Less,
      (_, Self::Changed(_)) => cmp::Ordering::Greater,

      // Added comes before Removed
      (Self::Added, Self::Removed) => cmp::Ordering::Less,
      (Self::Removed, Self::Added) => cmp::Ordering::Greater,
    }
  }
}

/// Documents if the derivation is a system package and if
/// it was added / removed as such.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivationSelectionStatus {
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

/// Writes a package diff between two paths to the provided writer.
///
/// This function queries the dependencies and system derivations of the
/// provided paths, and then generates and renders a diff between them.
///
/// # Returns
///
/// Returns the number of package diffs written.
///
/// # Errors
///
/// Returns an error if:
/// - Failed to connect to the store
/// - Failed to query dependencies or system derivations
/// - Failed to write to the output
pub fn write_package_diff(
  writer: &mut impl fmt::Write,
  path_old: &Path,
  path_new: &Path,
) -> Result<usize> {
  let connection = store::connect()?;

  // Query dependencies for old path
  let paths_old = connection
    .query_dependents(path_old)
    .with_context(|| {
      format!("failed to query dependencies of '{}'", path_old.display())
    })?
    .map(|(_, path)| path);

  // Query dependencies for new path
  let paths_new = connection
    .query_dependents(path_new)
    .with_context(|| {
      format!("failed to query dependencies of '{}'", path_new.display())
    })?
    .map(|(_, path)| path);

  // Query system derivations for old path
  let system_derivations_old = connection
    .query_system_derivations(path_old)
    .with_context(|| {
      format!(
        "failed to query system derivations of '{}'",
        path_old.display()
      )
    })?
    .map(|(_, path)| path);

  // Query system derivations for new path
  let system_derivations_new = connection
    .query_system_derivations(path_new)
    .with_context(|| {
      format!(
        "failed to query system derivations of '{}'",
        path_new.display()
      )
    })?
    .map(|(_, path)| path);

  writeln!(writer)?;

  // Generate and write the diff
  write_packages_diffln(
    writer,
    paths_old,
    paths_new,
    system_derivations_old,
    system_derivations_new,
  )
  .map_err(Error::from)
}

/// Writes a package diff between two paths to the provided writer.
///
/// This function is deprecated. Use [`write_package_diff`] instead.
/// # Errors
///
/// Returns an error if:
/// - Failed to connect to the store
/// - Failed to query dependencies or system derivations
/// - Failed to write to the output
#[deprecated(since = "1.4.0", note = "Use `write_package_diff` instead")]
pub fn write_paths_diffln(
  writer: &mut impl fmt::Write,
  path_old: &Path,
  path_new: &Path,
) -> Result<usize> {
  write_package_diff(writer, path_old, path_new)
}

/// Computes the Levenshtein distance between two slices using dynamic
/// programming.
///
/// Uses a standard implementation of the algorithm with O(min(m,n)) space
/// complexity and O(m*n) time complexity, where m and n are the lengths of
/// the input slices.
pub fn levenshtein<T: Eq>(from: &[T], to: &[T]) -> usize {
  let (from_len, to_len) = (from.len(), to.len());

  if from_len == 0 {
    return to_len;
  }
  if to_len == 0 {
    return from_len;
  }

  if from_len == to_len && from.iter().zip(to).all(|(a, b)| a == b) {
    return 0;
  }

  // Only special case for single character strings
  if from_len == 1 && to_len == 1 {
    return usize::from(from[0] != to[0]);
  }

  // Ensure we use the minimum required memory by making 'from' the shorter
  // slice
  if from_len > to_len {
    return levenshtein(to, from);
  }

  // We only need two rows - previous and current
  let mut prev_row = Vec::with_capacity(to_len + 1);
  prev_row.extend(0..=to_len);

  let mut curr_row = vec![0; to_len + 1];
  // Fill the matrix row by row
  for i in 1..=from_len {
    curr_row[0] = i;

    for j in 1..=to_len {
      // Cost is 0 if characters match, 1 otherwise
      let cost = usize::from(from[i - 1] != to[j - 1]);
      curr_row[j] = min(
        min(curr_row[j - 1] + 1, prev_row[j] + 1), // insertion and deletion
        prev_row[j - 1] + cost,                    // substitution
      );
    }

    // Early termination check - if this row's min value is too high
    if curr_row.iter().min().unwrap_or(&usize::MAX) > &to_len {
      return to_len;
    }

    // Swap rows
    swap(&mut prev_row, &mut curr_row);
  }

  prev_row[to_len]
}

/// Takes two lists of versions and tries to match them using the Hungarian
/// algorithm. The matching attempts to minimize the edit distance between
/// version pairs, which means:
///
/// 1. Versions with minimal edit distance are paired
/// 2. The natural ordering of versions is preserved where possible
///
/// Returns a vector of paired or unpaired versions (as `EitherOrBoth` enum).
pub fn match_version_lists<'a>(
  mut from: &'a [Version],
  mut to: &'a [Version],
) -> Vec<EitherOrBoth<&'a Version>> {
  // Early return for empty inputs
  if from.is_empty() {
    return to.iter().map(EitherOrBoth::Right).collect();
  }
  if to.is_empty() {
    return from.iter().map(EitherOrBoth::Left).collect();
  }

  // Quick path for common case - exact match
  if from.len() == 1 && to.len() == 1 && from[0] == to[0] {
    return vec![EitherOrBoth::Both(&from[0], &to[0])];
  }

  // Hungarian algorithm requires #rows <= #columns
  // Since the edit distance is symmetric, we can swap inputs if needed
  let swapped = if from.len() > to.len() {
    (to, from) = (from, to);
    true
  } else {
    false
  };

  // Pre-extract version components to avoid repetitive extraction
  let from_components: Vec<Vec<VersionComponent>> = from
    .iter()
    .map(|version| {
      version
        .into_iter()
        .filter_map(VersionPiece::component)
        .collect()
    })
    .collect();

  let to_components: Vec<Vec<VersionComponent>> = to
    .iter()
    .map(|version| {
      version
        .into_iter()
        .filter_map(VersionPiece::component)
        .collect()
    })
    .collect();

  let mut distances = Matrix::new(from.len(), to.len(), 0_i32);

  // Compute all distances directly into the matrix
  for i in 0..from.len() {
    for j in 0..to.len() {
      distances[(i, j)] =
        i32::try_from(levenshtein(&from_components[i], &to_components[j]))
          .unwrap_or_else(|err| {
            log::warn!("Distance must fit in i32: {err}");
            i32::MAX
          });
    }
  }

  // Apply Hungarian algorithm to find optimal pairings
  let (_cost, matchings) =
    kuhn_munkres::kuhn_munkres_min::<i32, Matrix<i32>>(&distances);

  // Process matched pairs
  let mut remaining = (0..to.len()).collect::<HashSet<usize>>();
  let mut pairings =
    Vec::<EitherOrBoth<&Version>>::with_capacity(from.len() + to.len());

  for (i, j) in matchings.into_iter().enumerate() {
    pairings.push(EitherOrBoth::Both(&from[i], &to[j]));
    remaining.remove(&j);
  }

  // Add unmatched items from 'to' list
  if !remaining.is_empty() {
    let mut remaining = remaining.iter().map(|&j| &to[j]).collect::<Vec<_>>();
    remaining.sort_unstable();
    pairings.extend(remaining.into_iter().map(EitherOrBoth::Right));
  }

  // Restore original ordering if we swapped the inputs
  if swapped {
    pairings = pairings.into_iter().map(EitherOrBoth::flip).collect();
  }

  pairings
}

/// Takes a list of versions which may contain duplicates and deduplicates it by
/// replacing multiple occurrences of an element with the same element plus the
/// amount it occurs.
///
/// This function sorts the input vector, counts consecutive identical versions,
/// and replaces them with a single version with count notation (e.g., "2.3
/// ×3"). Time complexity is O(n log n) due to sorting.
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
  // Early return for input <= 1.
  if versions.len() <= 1 {
    return;
  }

  // First sort to group identical versions together
  versions.sort_unstable();

  // Avoid reallocations by reusing the existing vector
  let mut result_index = 0;
  let mut count: usize = 1;

  // Process all but the last element
  for i in 1..versions.len() {
    if versions[i].name == versions[result_index].name {
      // Same version, increment count
      count += 1;
    } else {
      // Different version, finalize the previous one with count
      versions[result_index].amount = count;

      // Move to next position in result
      result_index += 1;
      versions.swap(result_index, i);
      count = 1;
    }
  }

  // Finalize the last group
  versions[result_index].amount = count;
  result_index += 1;

  // Truncate the vector to the actual number of unique versions
  versions.truncate(result_index);
}

/// Entry point for writing package differences.
///
/// # Errors
///
/// Returns an error if it fails writing to the `writer`
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

  let mut diffs = generate_diffs_from_paths(paths_map);
  add_selection_status(&mut diffs, &sys_old_set, &sys_new_set);

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

/// Collects package names from system paths
///
/// Takes an iterator of store paths and extracts the package names,
/// filtering out any that cannot be parsed. Logs warnings for parse failures.
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

/// Collects and organizes versions from old and new paths
///
/// Creates a mapping from package names to their versions in old and new paths.
/// For each package, stores a tuple of (`old_versions`, `new_versions`).
/// Handles parsing errors by logging warnings and skipping problematic entries.
fn collect_path_versions(
  old: impl Iterator<Item = StorePath>,
  new: impl Iterator<Item = StorePath>,
) -> HashMap<String, (Vec<Version>, Vec<Version>)> {
  let mut paths = HashMap::<String, (Vec<Version>, Vec<Version>)>::new();

  // Helper function to add a version to the appropriate list
  let add_version =
    |path: StorePath,
     is_old: bool,
     paths: &mut HashMap<String, (Vec<Version>, Vec<Version>)>| {
      match path.parse_name_and_version() {
        Ok((name, version)) => {
          let entry = paths
            .entry(name.into())
            .or_insert_with(|| (Vec::new(), Vec::new()));

          let list = if is_old { &mut entry.0 } else { &mut entry.1 };

          list.push(
            version.unwrap_or_else(|| Version::from("<none>".to_owned())),
          );
        },
        Err(err) => log::warn!("error parsing path: {err}"),
      }
    };

  // Process old paths
  for path in old {
    add_version(path, true, &mut paths);
  }

  // Process new paths
  for path in new {
    add_version(path, false, &mut paths);
  }

  paths
}

/// Renders a collection of diffs to the writer
///
/// Formats and writes the diffs in sections (CHANGED, ADDED, REMOVED),
/// including status indicators, package names, and version differences.
///
/// Returns the number of diffs rendered on success.
fn render_diffs(
  writer: &mut impl fmt::Write,
  diffs: &[Diff],
) -> Result<usize, fmt::Error> {
  // Calculate width needed for aligning package names
  let name_width = diffs
    .iter()
    .map(|diff| diff.name.width())
    .max()
    .unwrap_or(0)
    + 1;
  let mut last_status = None::<DiffStatus>;

  for diff in diffs {
    // Print section header when status changes
    if last_status.is_none_or(|ls| ls.cmp(&diff.status) != cmp::Ordering::Equal)
    {
      // Add blank line between sections (except before first section)
      if last_status.is_some() {
        writeln!(writer)?;
      }

      // Format and write the section header
      let header = match diff.status {
        DiffStatus::Changed(_) => "CHANGED",
        DiffStatus::Added => "ADDED",
        DiffStatus::Removed => "REMOVED",
      }
      .bold();

      writeln!(writer, "{header}")?;
      last_status = Some(diff.status);
    }

    // Format package info with status indicators
    let status_char = diff.status.char();
    let sel_char = diff.selection.char();
    let name_painted = diff.name.paint(sel_char.style);

    // Write package name with indicators
    write!(
      writer,
      "[{status_char}{sel_char}] {name_painted:<name_width$}"
    )?;

    // Format and write version differences
    let (old_str, new_str) =
      fmt_version_diffs(&diff.old, &diff.new, diff.has_common_versions)?;
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
///
/// This function:
/// 1. Matches old and new versions using the Hungarian algorithm
/// 2. For each matched pair, formats the differences with appropriate colors
/// 3. Handles unmatched versions in either list
///
/// Returns a tuple of formatted strings for the old and new versions.
fn fmt_version_diffs(
  old_versions: &[Version],
  new_versions: &[Version],
  has_common_versions: bool,
) -> Result<(String, String), fmt::Error> {
  // Pre-allocate strings with reasonable capacity
  let mut old_acc = String::with_capacity(
    old_versions
      .iter()
      .fold(0, |acc, version| acc + version.name.len() + 2),
  );
  let mut new_acc = String::with_capacity(
    new_versions
      .iter()
      .fold(0, |acc, version| acc + version.name.len() + 2),
  );

  let mut old_wrote = false;
  let mut new_wrote = false;

  // Helper function to append comma separators when needed
  let append_sep = |acc: &mut String, wrote: &mut bool| {
    if *wrote {
      write!(acc, ", ")
    } else {
      *wrote = true;
      Ok(())
    }
  };

  #[expect(clippy::redundant_closure_for_method_calls)]
  for diff in match_version_lists(old_versions, new_versions) {
    match diff {
      EitherOrBoth::Left(old) => {
        append_sep(&mut old_acc, &mut old_wrote)?;
        for comp in old {
          write_version_piece(&mut old_acc, &comp, |c| c.red())?;
        }
      },
      EitherOrBoth::Right(new) => {
        append_sep(&mut new_acc, &mut new_wrote)?;
        for comp in new {
          write_version_piece(&mut new_acc, &comp, |c| c.green())?;
        }
      },
      EitherOrBoth::Both(old, new) => {
        if old == new {
          continue;
        }

        append_sep(&mut old_acc, &mut old_wrote)?;
        append_sep(&mut new_acc, &mut new_wrote)?;

        fmt_single_version_diff(&mut old_acc, &mut new_acc, old, new)?;
      },
    }
  }
  if has_common_versions {
    let others_str = "<others>".blue().italic().to_string();
    append_sep(&mut old_acc, &mut old_wrote)?;
    append_sep(&mut new_acc, &mut new_wrote)?;
    write!(old_acc, "{others_str}")?;
    write!(new_acc, "{others_str}")?;
  }

  Ok((old_acc, new_acc))
}

/// Writes a version piece to a string buffer with the specified styling.
///
/// Components (like version numbers) get styled according to the provided style
/// function. Separators (like dots, dashes) are written as-is without styling.
///
/// # Parameters
/// * `buf` - The string buffer to write to
/// * `piece` - The version piece to write
/// * `style` - A function that applies a style to the version component
fn write_version_piece(
  buf: &mut String,
  piece: &VersionPiece,
  style: impl Fn(Painted<&str>) -> Painted<&str>,
) -> fmt::Result {
  match *piece {
    VersionPiece::Component(component) => {
      write!(buf, "{}", style(Painted::new(*component)))
    },
    VersionPiece::Separator(separator) => write!(buf, "{separator}"),
  }
}

/// Handles the logic of comparing two specific versions:
/// 1. Finds common prefixes and suffixes, which are colored yellow.
/// 2. Compares the remaining middle parts, with removals in red and additions
///    in green.
fn fmt_single_version_diff(
  old_acc: &mut String,
  new_acc: &mut String,
  old_ver: &Version,
  new_ver: &Version,
) -> fmt::Result {
  // Process version differences
  // Convert versions to piece vectors
  let old_parts: Vec<_> = old_ver.into_iter().collect();
  let new_parts: Vec<_> = new_ver.into_iter().collect();

  // Early return for empty versions or identical versions with same amounts
  if (old_parts.is_empty() && new_parts.is_empty()) || (old_ver == new_ver) {
    return Ok(());
  }

  // Find common prefix length
  let prefix_len = old_parts
    .iter()
    .zip(&new_parts)
    .take_while(|&(old_part, new_part)| old_part == new_part)
    .count();

  // Get remaining parts after removing the common prefix
  let old_remainder = &old_parts[prefix_len..];
  let new_remainder = &new_parts[prefix_len..];

  // Find common suffix length (if there's anything left after prefix removal)
  let suffix_len = if !old_remainder.is_empty() && !new_remainder.is_empty() {
    old_remainder
      .iter()
      .rev()
      .zip(new_remainder.iter().rev())
      .take_while(|&(old_part, new_part)| old_part == new_part)
      .count()
  } else {
    0
  };

  // Get the three sections: prefix, diff, and suffix
  let prefix = &old_parts[..prefix_len];
  let old_diff_end = old_parts.len() - suffix_len;
  let new_diff_end = new_parts.len() - suffix_len;

  let old_diff = &old_parts[prefix_len..old_diff_end];
  let new_diff = &new_parts[prefix_len..new_diff_end];
  let suffix = if suffix_len > 0 {
    &old_parts[old_diff_end..]
  } else {
    &[]
  };

  // Write common prefix (yellow)
  #[expect(clippy::redundant_closure_for_method_calls)]
  for piece in prefix {
    write_version_piece(old_acc, piece, |c| c.yellow())?;
    write_version_piece(new_acc, piece, |c| c.yellow())?;
  }

  // Write differing middle parts (red/green)
  for pair in Itertools::zip_longest(old_diff.iter(), new_diff.iter()) {
    #[expect(clippy::redundant_closure_for_method_calls)]
    match pair {
      EitherOrBoth::Left(old) => {
        write_version_piece(old_acc, old, |c| c.red())?;
      },
      EitherOrBoth::Right(new) => {
        write_version_piece(new_acc, new, |c| c.green())?;
      },
      EitherOrBoth::Both(old, new) => {
        fmt_version_piece_pair(old_acc, new_acc, old, new)?;
      },
    }
  }

  // Process common suffix
  // Write common suffix (yellow)
  #[expect(clippy::redundant_closure_for_method_calls)]
  for piece in suffix {
    write_version_piece(old_acc, piece, |c| c.yellow())?;
    write_version_piece(new_acc, piece, |c| c.yellow())?;
  }

  // Handle version amount differences
  if old_ver.amount == new_ver.amount {
    if old_ver.amount > 1 {
      // Same amount and greater than 1, display in yellow for both
      write!(old_acc, " ×{}", (old_ver.amount.to_string().yellow()))?;
      write!(new_acc, " ×{}", (new_ver.amount.to_string().yellow()))?;
    }
  } else {
    // Different amounts
    if old_ver.amount > 1 {
      write!(old_acc, " ×{}", (old_ver.amount.to_string().red()))?;
    }
    if new_ver.amount > 1 {
      write!(new_acc, " ×{}", (new_ver.amount.to_string().green()))?;
    }
  }

  Ok(())
}

/// Compares and formats two `VersionPieces`.
/// Format a pair of version pieces for diff display
///
/// For components, performs character-level diffing with special handling for
/// hashes. For separators or mixed types, simply colors them red/green.
/// Compares and formats two `VersionPieces` for displaying differences.
///
/// This function implements specialized character-by-character diffing for
/// version components with special handling for hash-like strings (like Nix
/// package hashes). For separators or mixed types, it simply colors the
/// old piece red and the new piece green.
///
/// Performance optimization is applied for very different components to avoid
/// expensive diffing when components are completely different.
fn fmt_version_piece_pair(
  old_acc: &mut String,
  new_acc: &mut String,
  old_piece: &VersionPiece,
  new_piece: &VersionPiece,
) -> fmt::Result {
  // Fast path for identical pieces
  if old_piece == new_piece {
    #[expect(clippy::redundant_closure_for_method_calls)]
    return {
      write_version_piece(old_acc, old_piece, |c| c.yellow())?;
      write_version_piece(new_acc, new_piece, |c| c.yellow())
    };
  }

  match (old_piece, new_piece) {
    // For version components, do character-level diffing
    (&VersionPiece::Component(old_c), &VersionPiece::Component(new_c)) => {
      // Skip detailed diffing for completely different components
      if old_c.len() > 20
        && new_c.len() > 20
        && old_c
          .chars()
          .zip(new_c.chars())
          .all(|(old_char, new_char)| old_char != new_char)
      {
        write!(old_acc, "{}", old_c.red())?;
        write!(new_acc, "{}", new_c.green())?;
        return Ok(());
      }

      let char_diffs = diff::chars(*old_c, *new_c);
      let mut diff_active = false;

      for res in char_diffs {
        match res {
          diff::Result::Both(left, right) => {
            // For matching characters, use yellow unless in hash diff mode
            if diff_active {
              write!(old_acc, "{}", left.red())?;
              write!(new_acc, "{}", right.green())?;
            } else {
              write!(old_acc, "{}", left.yellow())?;
              write!(new_acc, "{}", right.yellow())?;
            }
          },
          diff::Result::Left(left) => {
            // Character only in old version
            diff_active = true;
            write!(old_acc, "{}", left.red())?;
          },
          diff::Result::Right(right) => {
            // Character only in new version
            diff_active = true;
            write!(new_acc, "{}", right.green())?;
          },
        }
      }
    },
    // For separators or mixed types, color them red/green
    #[expect(clippy::redundant_closure_for_method_calls)]
    (old, new) => {
      write_version_piece(old_acc, old, |c| c.red())?;
      write_version_piece(new_acc, new, |c| c.green())?;
    },
  }
  Ok(())
}

/// Spawns a background task to compute the closure sizes required by
/// [`write_size_diffln`].
///
/// This function offloads the potentially expensive operation of calculating
/// closure sizes to a separate thread, allowing the main thread to continue
/// with other work while these calculations are performed.
///
/// # Returns
///
/// Returns a join handle that will resolve to the sizes when complete.
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

/// Writes a formatted size difference between two sizes to the provided writer.
///
/// This function displays both the absolute sizes (old → new) and the
/// difference between them, with appropriate coloring (red for size increase,
/// green for size decrease).
///
/// # Returns
///
/// Returns `Ok(())` when successful.
///
/// # Errors
///
/// Returns `Err` when writing to `writer` fails.
pub fn write_size_diff(
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

/// Writes a size diff between two sizes to the provided writer.
///
/// This function is deprecated. Use [`write_size_diff`] instead.
///
/// # Errors
///
/// Returns `Err` when writing to `writer` fails.
#[deprecated(since = "1.4.0", note = "Use `write_size_diff` instead")]
pub fn write_size_diffln(
  writer: &mut impl fmt::Write,
  size_old: Size,
  size_new: Size,
) -> fmt::Result {
  write_size_diff(writer, size_old, size_new)
}

/// Generates diff objects from a mapping of package names to old and new
/// versions
///
/// This function:
/// 1. Deduplicates versions in both old and new lists
/// 2. Determines the diff status (Added, Removed, or Changed)
/// 3. For Changed status, identifies if it's an upgrade, downgrade, or both
///
/// Returns a vector of Diff objects for all meaningful changes (filters out
/// unchanged items)
#[must_use]
pub fn generate_diffs_from_paths<S: BuildHasher>(
  paths: HashMap<String, (Vec<Version>, Vec<Version>), S>,
) -> Vec<Diff> {
  let mut result = Vec::with_capacity(paths.len());

  #[expect(clippy::iter_over_hash_type)]
  for (name, (mut old_versions, mut new_versions)) in paths {
    // Ensure versions are Sorted and Deduplicated
    deduplicate_versions(&mut old_versions);
    deduplicate_versions(&mut new_versions);

    // Using set-based comparison instead of the previous sorted-order
    // comparison. This fixes issues where versions with different suffixes
    // (like -dev vs -man) can be compared correctly regardless of their
    // lexicographical ordering. For example, when comparing:
    // "<none>, 258.2 x2, 258.2-man -> <none>, 258.2 x2, 258.2-dev, 258.2-man"
    // we need to recognize that 258.2-man appears in both sides despite
    // ordering differences.

    // Create clones for comparison
    let old_clone = old_versions.clone();
    let new_clone = new_versions.clone();

    // Create sets for efficient lookup
    let old_set: HashSet<Version> = old_clone.into_iter().collect();
    let new_set: HashSet<Version> = new_clone.into_iter().collect();

    // Find intersection (common versions)
    let common_set: HashSet<_> = old_set.intersection(&new_set).collect();
    let common_count = common_set.len();

    // Filter out versions that are in both lists to get unique ones
    let mut unique_old = Vec::new();
    let mut unique_new = Vec::new();

    for v in old_versions {
      if !new_set.contains(&v) {
        unique_old.push(v);
      }
    }

    for v in new_versions {
      if !old_set.contains(&v) {
        unique_new.push(v);
      }
    }

    let status =
      match (common_count, unique_old.is_empty(), unique_new.is_empty()) {
        (_, true, true) => continue,
        (0, true, false) => DiffStatus::Added,
        (0, false, true) => DiffStatus::Removed,
        (_, true, false) | (_, false, true) => {
          DiffStatus::Changed(Change::UpgradeDowngrade)
        },
        (_, false, false) => {
          if let Some(status) =
            determine_change_status(&unique_old, &unique_new)
          {
            status
          } else {
            continue;
          }
        },
      };

    result.push(Diff {
      name,
      old: unique_old,
      new: unique_new,
      status,
      selection: DerivationSelectionStatus::Unselected,
      has_common_versions: common_count > 0,
    });
  }

  result
}
/// Determines the change status for a package with both old and new versions.
///
/// Analyzes version pairs to determine if changes represent upgrades,
/// downgrades, or a mixture of both.
///
/// # Returns
///
/// Returns the appropriate `DiffStatus` or None if there's no meaningful change
fn determine_change_status(
  old_versions: &[Version],
  new_versions: &[Version],
) -> Option<DiffStatus> {
  if old_versions.is_empty() {
    return Some(DiffStatus::Changed(Change::Upgraded));
  }

  if new_versions.is_empty() {
    return Some(DiffStatus::Changed(Change::Downgraded));
  }

  if old_versions.len() == 1 && new_versions.len() == 1 {
    match old_versions[0].cmp(&new_versions[0]) {
      cmp::Ordering::Less => {
        return Some(DiffStatus::Changed(Change::Upgraded));
      },
      cmp::Ordering::Greater => {
        return Some(DiffStatus::Changed(Change::Downgraded));
      },
      cmp::Ordering::Equal => return None,
    }
  }

  let mut saw_upgrade = false;
  let mut saw_downgrade = false;

  // Compare versions to determine if we have upgrades, downgrades or both
  for ver_diff in match_version_lists(old_versions, new_versions) {
    match ver_diff {
      EitherOrBoth::Left(_) => saw_downgrade = true,
      EitherOrBoth::Right(_) => saw_upgrade = true,
      EitherOrBoth::Both(old, new) => {
        match old.cmp(new) {
          cmp::Ordering::Less => saw_upgrade = true,
          cmp::Ordering::Greater => saw_downgrade = true,
          cmp::Ordering::Equal => {}, // Shouldn't happen due to filtering above
        }
      },
    }

    // Early exit optimization
    if saw_upgrade && saw_downgrade {
      break;
    }
  }

  match (saw_upgrade, saw_downgrade) {
    (true, true) => Some(DiffStatus::Changed(Change::UpgradeDowngrade)),
    (true, false) => Some(DiffStatus::Changed(Change::Upgraded)),
    (false, true) => Some(DiffStatus::Changed(Change::Downgraded)),
    (false, false) => None, // No actual changes
  }
}

/// Adds selection status to each diff based on whether the package name
/// is present in the old and new system paths.
///
/// This determines if packages are system packages and if their status changed,
/// allowing the renderer to show appropriate indicators:
/// - `Selected` (*)      - System package in both old and new
/// - `NewlySelected` (+) - Dependency in old, system package in new
/// - `Unselected` (.)    - Dependency in both old and new
/// - `NewlyUnselected` (-) - System package in old, dependency in new
pub fn add_selection_status(
  diffs: &mut [Diff],
  system_paths_old: &HashSet<String>,
  system_paths_new: &HashSet<String>,
) {
  for diff in diffs {
    diff.selection = DerivationSelectionStatus::from_names(
      &diff.name,
      system_paths_old,
      system_paths_new,
    );
  }
}

#[cfg(test)]
mod tests {
  use std::collections::{
    HashMap,
    HashSet,
  };

  use crate::{
    diff::{
      Change,
      DerivationSelectionStatus,
      Diff,
      DiffStatus,
      add_selection_status,
      levenshtein,
      match_version_lists,
    },
    generate_diffs_from_paths,
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
      Version::new("5.116.0"),
      Version::new("5.116.0-bin"),
      Version::new("6.16.0"),
    ];
    let version_list_b = [Version::new("6.17.0")];

    let matched = match_version_lists(&version_list_a, &version_list_b);

    for version in matched {
      #[expect(clippy::print_stdout)]
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

  #[test]
  fn generate_diffs_from_paths_test() {
    use crate::version::Version;
    let mut paths: HashMap<String, (Vec<Version>, Vec<Version>)> =
      HashMap::new();

    let diff_1 = (vec![Version::new("1.1.0"), Version::new("1.3")], vec![
      Version::new("1.1.0"),
      Version::new("1.4"),
    ]);
    paths.insert("tmp".to_owned(), diff_1);
    let mut vec_1 = generate_diffs_from_paths(paths);
    add_selection_status(
      &mut vec_1,
      &HashSet::<String>::new(),
      &HashSet::<String>::new(),
    );
    let res_2 = Diff {
      name:                "tmp".to_owned(),
      old:                 vec![Version::new("1.3")],
      new:                 vec![Version::new("1.4")],
      status:              DiffStatus::Changed(Change::Upgraded),
      selection:           DerivationSelectionStatus::Unselected,
      has_common_versions: true,
    };
    assert_eq!(vec_1.first().unwrap(), &res_2);

    paths = HashMap::new();

    let diff_2 = (vec![Version::new("1.2.0"), Version::new("1.5")], vec![
      Version::new("1.2.0"),
    ]);
    paths.insert("tmp".to_owned(), diff_2);
    let mut vec_2 = generate_diffs_from_paths(paths);
    add_selection_status(
      &mut vec_2,
      &HashSet::<String>::new(),
      &HashSet::<String>::new(),
    );
    let res_2 = Diff {
      name:                "tmp".to_owned(),
      old:                 vec![Version::new("1.5")],
      new:                 vec![],
      status:              DiffStatus::Changed(Change::UpgradeDowngrade),
      selection:           DerivationSelectionStatus::Unselected,
      has_common_versions: true,
    };
    assert_eq!(vec_2.first().unwrap(), &res_2);
  }
}
