use std::path::PathBuf;

use eyre::{
  Result,
  WrapErr as _,
};
use serde::Serialize;

use crate::{
  diff::{
    Diff,
    add_selection_status,
    collect_path_versions,
    collect_system_names,
    create_backend,
  },
  generate_diffs_from_paths,
  store::StoreBackend as _,
};

pub fn display_diff(
  path_old: &PathBuf,
  path_new: &PathBuf,
  force_correctness: bool,
) -> Result<()> {
  let mut connection = create_backend(force_correctness);
  connection.connect()?;

  // Query dependencies for old path
  let paths_old = connection.query_dependents(path_old).with_context(|| {
    format!("failed to query dependencies of '{}'", path_old.display())
  })?;

  // Query dependencies for new path
  let paths_new = connection.query_dependents(path_new).with_context(|| {
    format!("failed to query dependencies of '{}'", path_new.display())
  })?;

  // Query system derivations for old path
  let system_derivations_old = connection
    .query_system_derivations(path_old)
    .with_context(|| {
      format!(
        "failed to query system derivations of '{}'",
        path_old.display()
      )
    })?;

  // Query system derivations for new path
  let system_derivations_new = connection
    .query_system_derivations(path_new)
    .with_context(|| {
      format!(
        "failed to query system derivations of '{}'",
        path_new.display()
      )
    })?;

  let paths_map = collect_path_versions(paths_old, paths_new);
  let sys_old_set = collect_system_names(system_derivations_old, "old");
  let sys_new_set = collect_system_names(system_derivations_new, "new");

  let mut diffs = generate_diffs_from_paths(paths_map);
  add_selection_status(&mut diffs, &sys_old_set, &sys_new_set);
  let size_old = connection.query_closure_size(path_old)?.bytes();
  let size_new = connection.query_closure_size(path_new)?.bytes();

  let json = report_to_json(JsonReport {
    diffs,
    size_old,
    size_new,
  })?;
  print!("{json}");
  Ok(())
}

#[derive(Serialize)]
pub struct JsonReport {
  /// package changes
  diffs:    Vec<Diff>,
  /// old closure size (in bytes)
  size_old: i64,
  /// new closure size (in bytes)
  size_new: i64,
}

pub fn report_to_json(diffs: JsonReport) -> Result<String> {
  serde_json::to_string(&diffs).map_err(|e| e.into())
}
