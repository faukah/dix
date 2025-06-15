use std::{
  cmp,
  collections::{
    HashMap,
    HashSet,
  },
  fmt::{
    self,
    Write as _,
  },
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
    #[expect(unreachable_patterns)]
    match (*self, *other) {
      (Self::Changed(_), Self::Changed(_)) => cmp::Ordering::Equal,
      (Self::Changed(_), _) => cmp::Ordering::Less,
      (_, Self::Changed(_)) => cmp::Ordering::Greater,

      (Self::Added, Self::Added) => cmp::Ordering::Equal,
      (Self::Added, _) => cmp::Ordering::Less,

      (Self::Removed, Self::Removed) => cmp::Ordering::Equal,
      (Self::Removed, _) => cmp::Ordering::Greater,
      (_, Self::Removed) => cmp::Ordering::Less,
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

  log::info!(
    "found {count}+ packages in old closure",
    count = paths_old.size_hint().0,
  );

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

  log::info!(
    "found {count}+ packages in new closure",
    count = paths_new.size_hint().0,
  );

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
    new = path_new.display(),
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
        log::debug!("parsed name: {name}");
        log::debug!("parsed version: {version:?}");

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
        log::debug!("parsed name: {name}");
        log::debug!("parsed version: {version:?}");

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

      let status = match (versions.old.len(), versions.new.len()) {
        (0, 0) => unreachable!(),
        (0, _) => DiffStatus::Added,
        (_, 0) => DiffStatus::Removed,
        _ => {
          let mut saw_upgrade = false;
          let mut saw_downgrade = false;

          for diff in
            Itertools::zip_longest(versions.old.iter(), versions.new.iter())
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

    for diff in Itertools::zip_longest(versions.old.iter(), versions.new.iter())
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
              Ok(old_comp) => write!(oldacc, "{old}", old = old_comp.red())?,
              Err(ignored) => write!(oldacc, "{ignored}")?,
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
              Ok(new_comp) => write!(newacc, "{new}", new = new_comp.green())?,
              Err(ignored) => write!(newacc, "{ignored}")?,
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

          for diff in Itertools::zip_longest(
            old_version.into_iter(),
            new_version.into_iter(),
          ) {
            match diff {
              EitherOrBoth::Left(old_comp) => {
                match old_comp {
                  Ok(old_comp) => {
                    write!(oldacc, "{old}", old = old_comp.red())?;
                  },
                  Err(ignored) => {
                    write!(oldacc, "{ignored}")?;
                  },
                }
              },

              EitherOrBoth::Right(new_comp) => {
                match new_comp {
                  Ok(new_comp) => {
                    write!(newacc, "{new}", new = new_comp.green())?;
                  },
                  Err(ignored) => {
                    write!(newacc, "{ignored}")?;
                  },
                }
              },

              EitherOrBoth::Both(old_comp, new_comp) => {
                match (old_comp, new_comp) {
                  (Ok(old_comp), Ok(new_comp)) => {
                    for char in diff::chars(*old_comp, *new_comp) {
                      match char {
                        diff::Result::Left(old_part) => {
                          write!(oldacc, "{old}", old = old_part.red())?;
                        },
                        diff::Result::Right(new_part) => {
                          write!(newacc, "{new}", new = new_part.green())?;
                        },

                        diff::Result::Both(old_part, new_part) => {
                          write!(oldacc, "{old}", old = old_part.yellow())?;
                          write!(newacc, "{new}", new = new_part.yellow())?;
                        },
                      }
                    }
                  },

                  (old_comp, new_comp) => {
                    match old_comp {
                      Ok(old_comp) => {
                        write!(oldacc, "{old}", old = old_comp.yellow())?;
                      },
                      Err(old_comp) => write!(oldacc, "{old_comp}")?,
                    }

                    match new_comp {
                      Ok(new_comp) => {
                        write!(newacc, "{new}", new = new_comp.yellow())?;
                      },
                      Err(new_comp) => write!(newacc, "{new_comp}")?,
                    }
                  },
                }
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
