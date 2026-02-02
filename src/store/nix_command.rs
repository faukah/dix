use std::{
  fmt::{
    self,
    Display,
  },
  path::{
    Path,
    PathBuf,
  },
  process::Command,
};

use eyre::{
  Context,
  Result,
  eyre,
};
use size::Size;

use crate::{
  StorePath,
  store::StoreBackend,
};

#[derive(Debug, Default)]
/// Uses nix commands to perform queries.
///
/// This is similar in implementation to the old `dix` in its early stages and
/// is supposed to be a final fallback if the direct queries on the database
/// fail. It is considerably slower than the direct queries and currently does
/// not support querying the whole dependency graph.
pub struct CommandBackend;

impl Display for CommandBackend {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "CommandBackend")
  }
}

fn nix_command_query<'a>(
  args: &'a [&'a str],
) -> Result<Box<dyn Iterator<Item = StorePath>>> {
  let command_str = format!("nix-store {}", args.join(" "));
  tracing::debug!(command = %command_str, "executing nix command");
  let references = Command::new("nix-store").args(args).output();

  let query = references?;
  tracing::trace!(command = %command_str, "nix command executed successfully");
  // We just collect into a vec, as this method of
  // querying data is slow anyways
  let mut paths = Vec::new();
  for line in str::from_utf8(&query.stdout)?.lines() {
    let path = StorePath::try_from(PathBuf::from(line)).context(eyre!(
      "encountered invalid path in nix command output: {line}"
    ))?;
    paths.push(path);
  }

  Ok(Box::new(paths.into_iter()))
}

impl<'a> StoreBackend<'a> for CommandBackend {
  /// Does nothing (we spawn a new process everytime).
  fn connect(&mut self) -> Result<()> {
    Ok(())
  }

  /// we don't really have a connection
  /// always returns true
  fn connected(&self) -> bool {
    true
  }

  /// there is nothing to close
  fn close(&mut self) -> Result<()> {
    Ok(())
  }

  fn query_closure_size(&self, path: &Path) -> Result<Size> {
    let cmd_res = Command::new("nix")
      .arg("path-info")
      .arg("--closure-size")
      .arg(path.join("sw"))
      .output()
      .wrap_err("Encountered error while executing nix command")?;
    let text = str::from_utf8(&cmd_res.stdout)?;
    if let Some(bytes_text) = text.split_whitespace().last()
      && let Ok(bytes) = bytes_text.parse::<u64>()
    {
      Ok(Size::from_bytes(bytes))
    } else {
      Err(eyre!("Unable to parse closure size from nix output"))
    }
  }

  fn query_system_derivations(
    &self,
    system: &Path,
  ) -> Result<Box<dyn Iterator<Item = StorePath> + '_>> {
    nix_command_query(&[
      "--query",
      "--references",
      &*system.join("sw").to_string_lossy(),
    ])
  }

  fn query_dependents(
    &self,
    path: &Path,
  ) -> Result<Box<dyn Iterator<Item = StorePath> + '_>> {
    nix_command_query(&["--query", "--requisites", &*path.to_string_lossy()])
  }
}
