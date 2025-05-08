use std::{
  fmt,
  io,
};

use rustc_hash::{
  FxBuildHasher,
  FxHashMap,
};
use yansi::Paint as _;

use crate::{
  StorePath,
  Version,
};

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

impl fmt::Display for DiffStatus {
  fn fmt(&self, writer: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
      writer,
      "[{letter}]",
      letter = match *self {
        DiffStatus::Added => "A".green(),
        DiffStatus::Removed => "R".red(),
        DiffStatus::Changed => "C".yellow(),
      },
    )
  }
}

pub fn diff<'a>(
  writer: &mut dyn io::Write,
  paths_old: impl Iterator<Item = &'a StorePath>,
  paths_new: impl Iterator<Item = &'a StorePath>,
) -> io::Result<()> {
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
    .filter_map(|(name, versions)| {
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

  for (name, _versions, status) in diffs {
    write!(writer, "{status} {name}")?;
    writeln!(writer)?;
  }

  Ok(())
}
