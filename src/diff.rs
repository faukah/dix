use std::fmt::{
  self,
  Write as _,
};

use itertools::{
  EitherOrBoth,
  Itertools,
};
use ref_cast::RefCast as _;
use rustc_hash::{
  FxBuildHasher,
  FxHashMap,
};
use unicode_width::UnicodeWidthStr as _;
use yansi::Paint as _;

use crate::{
  StorePath,
  Version,
};

#[derive(Debug, Default)]
struct Diff<T> {
  old: T,
  new: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DiffStatus {
  Added,
  Removed,
  Changed,
}

impl DiffStatus {
  fn char(self) -> impl fmt::Display {
    match self {
      DiffStatus::Added => "A".green(),
      DiffStatus::Removed => "R".red(),
      DiffStatus::Changed => "C".yellow(),
    }
  }
}

pub fn write_diffln<'a>(
  writer: &mut dyn fmt::Write,
  paths_old: impl Iterator<Item = &'a StorePath>,
  paths_new: impl Iterator<Item = &'a StorePath>,
) -> fmt::Result {
  let mut paths =
    FxHashMap::<&str, Diff<Vec<Option<&Version>>>>::with_hasher(FxBuildHasher);

  for path in paths_old {
    match path.parse_name_and_version() {
      Ok((name, version)) => {
        paths.entry(name).or_default().old.push(version);
      },

      Err(error) => {
        log::info!("error parsing old path name and version: {error}");
      },
    }
  }

  for path in paths_new {
    match path.parse_name_and_version() {
      Ok((name, version)) => {
        paths.entry(name).or_default().new.push(version);
      },

      Err(error) => {
        log::info!("error parsing new path name and version: {error}");
      },
    }
  }

  let mut diffs = paths
    .into_iter()
    .filter_map(|(name, mut versions)| {
      versions.old.sort_unstable();
      versions.new.sort_unstable();

      let status = match (versions.old.len(), versions.new.len()) {
        (0, 0) => unreachable!(),
        (0, _) => DiffStatus::Removed,
        (_, 0) => DiffStatus::Added,
        (..) if versions.old != versions.new => DiffStatus::Changed,
        (..) => return None,
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

  for (name, versions, status) in diffs {
    if last_status != Some(status) {
      writeln!(
        writer,
        "{nl}{status}",
        nl = if last_status.is_some() { "\n" } else { "" },
        status = match status {
          DiffStatus::Added => "ADDED",
          DiffStatus::Removed => "REMOVED",
          DiffStatus::Changed => "CHANGED",
        }
        .bold(),
      )?;

      last_status = Some(status);
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
        EitherOrBoth::Right(old_version) => {
          if oldwrote {
            write!(oldacc, ", ")?;
          } else {
            write!(oldacc, " ")?;
            oldwrote = true;
          }

          for old_comp in old_version.unwrap_or(Version::ref_cast("<none>")) {
            match old_comp {
              Ok(old_comp) => write!(oldacc, "{old}", old = old_comp.red())?,
              Err(ignored) => write!(oldacc, "{ignored}")?,
            }
          }
        },

        // I have no idea why itertools is returning `versions.new` in `Left`.
        EitherOrBoth::Left(new_version) => {
          if newwrote {
            write!(newacc, ", ")?;
          } else {
            write!(newacc, " ")?;
            newwrote = true;
          }

          for new_comp in new_version.unwrap_or(Version::ref_cast("<none>")) {
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

          let old_version = old_version.unwrap_or(Version::ref_cast("<none>"));
          let new_version = new_version.unwrap_or(Version::ref_cast("<none>"));

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
              EitherOrBoth::Right(old_comp) => {
                match old_comp {
                  Ok(old_comp) => {
                    write!(oldacc, "{old}", old = old_comp.red())?;
                  },
                  Err(ignored) => {
                    write!(oldacc, "{ignored}")?;
                  },
                }
              },

              EitherOrBoth::Left(new_comp) => {
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
                if let Err(ignored) = old_comp {
                  write!(oldacc, "{ignored}")?;
                }

                if let Err(ignored) = new_comp {
                  write!(newacc, "{ignored}")?;
                }

                if let (Ok(old_comp), Ok(new_comp)) = (old_comp, new_comp) {
                  if old_comp == new_comp {
                    write!(oldacc, "{old}", old = old_comp.yellow())?;
                    write!(newacc, "{new}", new = new_comp.yellow())?;
                  } else {
                    write!(oldacc, "{old}", old = old_comp.red())?;
                    write!(newacc, "{new}", new = new_comp.green())?;
                  }
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
        " →"
      } else {
        ""
      }
    )?;

    writeln!(writer)?;
  }

  Ok(())
}
