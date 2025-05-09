use std::{
  fmt::{
    self,
    Write as _,
  },
  io::{
    self,
    Write as _,
  },
  path::PathBuf,
  process,
  thread,
};

use anyhow::{
  Context as _,
  Error,
  Result,
  anyhow,
};
use clap::Parser as _;
use yansi::Paint as _;

struct WriteFmt<W: io::Write>(W);

impl<W: io::Write> fmt::Write for WriteFmt<W> {
  fn write_str(&mut self, string: &str) -> fmt::Result {
    self.0.write_all(string.as_bytes()).map_err(|_| fmt::Error)
  }
}

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Cli {
  old_path: PathBuf,
  new_path: PathBuf,

  #[command(flatten)]
  verbose: clap_verbosity_flag::Verbosity,
}

fn real_main() -> Result<()> {
  let Cli {
    old_path,
    new_path,
    verbose,
  } = Cli::parse();

  yansi::whenever(yansi::Condition::TTY_AND_COLOR);

  env_logger::Builder::new()
    .filter_level(verbose.log_level_filter())
    .init();

  // Handle to the thread collecting closure size information.
  // We do this as early as possible because Nix is slow.
  let closure_size_handle = {
    log::debug!("calculating closure sizes in background");

    let old_path = old_path.clone();
    let new_path = new_path.clone();

    thread::spawn(move || {
      let mut connection = dix::store::connect()?;

      Ok::<_, Error>((
        connection.query_closure_size(&old_path)?,
        connection.query_closure_size(&new_path)?,
      ))
    })
  };

  let mut connection = dix::store::connect()?;

  let paths_old =
    connection.query_depdendents(&old_path).with_context(|| {
      format!(
        "failed to query dependencies of path '{path}'",
        path = old_path.display()
      )
    })?;

  log::debug!(
    "found {count} packages in old closure",
    count = paths_old.len(),
  );

  let paths_new =
    connection.query_depdendents(&new_path).with_context(|| {
      format!(
        "failed to query dependencies of path '{path}'",
        path = new_path.display()
      )
    })?;

  log::debug!(
    "found {count} packages in new closure",
    count = paths_new.len(),
  );

  drop(connection);

  let mut out = WriteFmt(io::stdout());

  writeln!(
    out,
    "{arrows} {old_path}",
    arrows = "<<<".bold(),
    old_path = old_path.display(),
  )?;
  writeln!(
    out,
    "{arrows} {new_path}",
    arrows = ">>>".bold(),
    new_path = new_path.display(),
  )?;

  writeln!(out)?;

  #[expect(clippy::pattern_type_mismatch)]
  dix::write_diffln(
    &mut out,
    paths_old.iter().map(|(_, path)| path),
    paths_new.iter().map(|(_, path)| path),
  )?;

  let (closure_size_old, closure_size_new) = closure_size_handle
    .join()
    .map_err(|_| anyhow!("failed to get closure size due to thread error"))??;

  let size_old = size::Size::from_bytes(closure_size_old);
  let size_new = size::Size::from_bytes(closure_size_new);
  let size_diff = size_new - size_old;

  writeln!(out)?;

  writeln!(
    out,
    "{header}: {size_old} → {size_new}",
    header = "SIZE".bold(),
    size_old = size_old.red(),
    size_new = size_new.green(),
  )?;

  writeln!(
    out,
    "{header}: {size_diff}",
    header = "DIFF".bold(),
    size_diff = if size_diff.bytes() > 0 {
      size_diff.green()
    } else {
      size_diff.red()
    },
  )?;

  Ok(())
}

#[allow(clippy::allow_attributes, clippy::exit)]
fn main() {
  let Err(error) = real_main() else {
    return;
  };

  let mut err = io::stderr();

  let mut message = String::new();
  let mut chain = error.chain().rev().peekable();

  while let Some(error) = chain.next() {
    let _ = write!(
      err,
      "{header} ",
      header = if chain.peek().is_none() {
        "error:"
      } else {
        "cause:"
      }
      .red()
      .bold(),
    );

    String::clear(&mut message);
    let _ = write!(message, "{error}");

    let mut chars = message.char_indices();

    let _ = match (chars.next(), chars.next()) {
      (Some((_, first)), Some((second_start, second)))
        if second.is_lowercase() =>
      {
        writeln!(
          err,
          "{first_lowercase}{rest}",
          first_lowercase = first.to_lowercase(),
          rest = &message[second_start..],
        )
      },

      _ => {
        writeln!(err, "{message}")
      },
    };
  }

  process::exit(1);
}
