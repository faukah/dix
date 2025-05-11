use std::{
  cmp,
  collections::HashMap,
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
use yansi::Paint as _;

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
enum DiffStatus {
  Changed,
  Upgraded,
  Downgraded,
  Added,
  Removed,
}

impl DiffStatus {
  fn char(self) -> impl fmt::Display {
    match self {
      Self::Changed => "C".yellow().bold(),
      Self::Upgraded => "U".bright_cyan().bold(),
      Self::Downgraded => "D".magenta().bold(),
      Self::Added => "A".green().bold(),
      Self::Removed => "R".red().bold(),
    }
  }
}
impl cmp::Ord for DiffStatus {
  fn cmp(&self, other: &Self) -> cmp::Ordering {
    use DiffStatus::{
      Added,
      Changed,
      Downgraded,
      Removed,
      Upgraded,
    };
    #[expect(clippy::match_same_arms)]
    match (*self, *other) {
      // Changeds get displayed earlier than adds or removes.
      (Changed | Upgraded | Downgraded, Removed | Added) => cmp::Ordering::Less,
      // adds get displayed before removes
      (Added, Removed) => cmp::Ordering::Less,
      (Removed | Added, _) => cmp::Ordering::Greater,
      _ => cmp::Ordering::Equal,
    }
  }
}

impl PartialOrd for DiffStatus {
  fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
    Some(self.cmp(other))
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

  let paths_old = connection.query_dependents(path_old).with_context(|| {
    format!(
      "failed to query dependencies of path '{path}'",
      path = path_old.display()
    )
  })?;

  log::info!(
    "found {count} packages in old closure",
    count = paths_old.len(),
  );

  let paths_new = connection.query_dependents(path_new).with_context(|| {
    format!(
      "failed to query dependencies of path '{path}'",
      path = path_new.display()
    )
  })?;
  log::info!(
    "found {count} packages in new closure",
    count = paths_new.len(),
  );

  drop(connection);

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

  #[expect(clippy::pattern_type_mismatch)]
  Ok(write_packages_diffln(
    writer,
    paths_old.iter().map(|(_, path)| path),
    paths_new.iter().map(|(_, path)| path),
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
fn write_packages_diffln<'a>(
  writer: &mut impl fmt::Write,
  paths_old: impl Iterator<Item = &'a StorePath>,
  paths_new: impl Iterator<Item = &'a StorePath>,
) -> Result<usize, fmt::Error> {
  let mut paths = HashMap::<&str, Diff<Vec<Version>>>::new();

  for path in paths_old {
    match path.parse_name_and_version() {
      Ok((name, version)) => {
        log::debug!("parsed name: {name}");
        log::debug!("parsed version: {version:?}");

        paths
          .entry(name)
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
          .entry(name)
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

          match (saw_upgrade, saw_downgrade) {
            (true, true) => DiffStatus::Changed,
            (true, false) => DiffStatus::Upgraded,
            (false, true) => DiffStatus::Downgraded,
            _ => return None,
          }
        },
      };

      Some((name, versions, status))
    })
    .collect::<Vec<_>>();

  diffs.sort_by(|&(a_name, _, a_status), &(b_name, _, b_status)| {
    a_status.cmp(&b_status).then_with(|| a_name.cmp(b_name))
  });

  let name_width = diffs
    .iter()
    .map(|&(name, ..)| name.width())
    .max()
    .unwrap_or(0);

  let mut last_status = None::<DiffStatus>;

  for &(name, ref versions, status) in &diffs {
    use DiffStatus::{
      Added,
      Changed,
      Downgraded,
      Removed,
      Upgraded,
    };
    let merged_status = if let Downgraded | Upgraded = status {
      Changed
    } else {
      status
    };

    if last_status != Some(merged_status) {
      writeln!(
        writer,
        "{nl}{status}",
        nl = if last_status.is_some() { "\n" } else { "" },
        status = match merged_status {
          Changed => "CHANGED",
          Upgraded | Downgraded => unreachable!(),
          Added => "ADDED",
          Removed => "REMOVED",
        }
        .bold(),
      )?;

      last_status = Some(merged_status);
    }

    write!(
      writer,
      "[{status}] {name:<name_width$}",
      status = status.char()
    )?;

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
