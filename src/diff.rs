use std::fmt::{
  self,
  Write as _,
};

use itertools::EitherOrBoth;
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

const HEADER_STYLE: yansi::Style = yansi::Style::new().bold().underline();

#[derive(Default)]
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

pub fn diff<'a>(
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
      last_status = Some(status);
      HEADER_STYLE.fmt_prefix(writer)?;
      writeln!(writer, "{status:?} packages:")?;
      HEADER_STYLE.fmt_suffix(writer)?;
    }

    write!(
      writer,
      "[{status}] {name:<name_width$}",
      status = status.char()
    )?;

    let mut oldacc = String::new();
    let mut newacc = String::new();

    for diff in itertools::Itertools::zip_longest(
      versions.old.iter(),
      versions.new.iter(),
    ) {
      match diff {
        // I have no idea why itertools is returning `versions.new` in `Left`.
        EitherOrBoth::Left(new) => {
          write!(
            newacc,
            " {new}",
            new = new.unwrap_or(Version::ref_cast("<none>")).green()
          )?;
        },

        EitherOrBoth::Both(old, new) => {
          let (old, new) = Version::diff(
            old.unwrap_or(Version::ref_cast("<none>")),
            new.unwrap_or(Version::ref_cast("<none>")),
          );

          write!(oldacc, " {old}")?;
          write!(newacc, " {new}")?;
        },

        EitherOrBoth::Right(old) => {
          write!(
            oldacc,
            " {old}",
            old = old.unwrap_or(Version::ref_cast("<none>")).red()
          )?;
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
