use std::{
  env,
  fmt::{
    self,
    Write as _,
  },
  fs,
  io::{
    self,
    IsTerminal as _,
  },
  path::PathBuf,
};

use clap::Parser as _;
use eyre::eyre;
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

  /// Controls when to use color.
  #[arg(
      long,
      default_value_t = clap::ColorChoice::Auto,
      value_name = "WHEN",
      global = true,
  )]
  color: clap::ColorChoice,

  /// Fall back to a backend that is focused solely on absolutely guaranteeing
  /// correct results at the cost of memory usage and query speed.
  ///
  /// This is relevant if the output of dix is to be used for more
  /// critical applications and not just as human-readable overview.
  ///
  /// In the vast, vast majority of cases, the default backend should be
  /// sufficient.
  #[arg(long, default_value_t = false, global = true)]
  force_correctness: bool,
}

fn main() -> eyre::Result<()> {
  let Cli {
    old_path,
    new_path,
    verbose,
    color,
    force_correctness,
  } = Cli::parse();

  // Validate that both paths exist before proceeding
  if !old_path.exists() {
    return Err(eyre!(
      "old profile path does not exist: {}",
      old_path.display()
    ));
  }
  if !new_path.exists() {
    return Err(eyre!(
      "new profile path does not exist: {}",
      new_path.display()
    ));
  }

  yansi::whenever(match color {
    clap::ColorChoice::Auto => yansi::Condition::from(should_style),
    clap::ColorChoice::Always => yansi::Condition::ALWAYS,
    clap::ColorChoice::Never => yansi::Condition::NEVER,
  });

  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::builder()
        .with_default_directive(match verbose.log_level_filter() {
          clap_verbosity_flag::log::LevelFilter::Off => {
            tracing::Level::ERROR.into()
          },
          clap_verbosity_flag::log::LevelFilter::Error => {
            tracing::Level::ERROR.into()
          },
          clap_verbosity_flag::log::LevelFilter::Warn => {
            tracing::Level::WARN.into()
          },
          clap_verbosity_flag::log::LevelFilter::Info => {
            tracing::Level::INFO.into()
          },
          clap_verbosity_flag::log::LevelFilter::Debug => {
            tracing::Level::DEBUG.into()
          },
          clap_verbosity_flag::log::LevelFilter::Trace => {
            tracing::Level::TRACE.into()
          },
        })
        .from_env_lossy(),
    )
    .with_target(false)
    .init();
  if force_correctness {
    tracing::warn!(
      "Falling back to slower but more robust backends (force_correctness is \
       set)."
    );
  }

  let mut out = WriteFmt(io::stdout());

  writeln!(
    out,
    "{arrows} {old}",
    arrows = "<<<".bold(),
    old = old_path.display(),
  )?;
  writeln!(
    out,
    "{arrows} {new}",
    arrows = ">>>".bold(),
    new = fs::canonicalize(&new_path)
      .unwrap_or_else(|_| new_path.clone())
      .display(),
  )?;

  // Handle to the thread collecting closure size information.
  let closure_size_handle =
    dix::spawn_size_diff(old_path.clone(), new_path.clone(), force_correctness);

  let wrote =
    dix::write_package_diff(&mut out, &old_path, &new_path, force_correctness)?;

  let (size_old, size_new) = closure_size_handle.join().map_err(|_| {
    eyre!(
      "failed to get closure size due to thread
  error"
    )
  })??;

  if wrote > 0 {
    writeln!(out)?;
  }

  dix::write_size_diff(&mut out, size_old, size_new)?;

  Ok(())
}

// https://bixense.com/clicolors/
fn should_style() -> bool {
  // If NO_COLOR is set and is not empty, don't style.
  if let Some(value) = env::var_os("NO_COLOR")
    && !value.is_empty()
  {
    return false;
  }

  // If CLICOLOR is set and is 0, don't style.
  if let Some(value) = env::var_os("CLICOLOR")
    && value == "0"
  {
    return false;
  }

  // If CLICOLOR_FORCE is set and not 0, always style.
  if let Some(value) = env::var_os("CLICOLOR_FORCE")
    && value != "0"
  {
    return true;
  }

  // Style if it is a terminal.
  io::stdout().is_terminal()
}
