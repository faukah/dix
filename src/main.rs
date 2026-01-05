use std::{
  fmt::{
    self,
    Write as _,
  },
  fs,
  io::{
    self,
    Write as _,
  },
  path::PathBuf,
  process,
};

use anyhow::{
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
    .format(|out, arguments| {
      let header = match arguments.level() {
        log::Level::Error => "error:".red(),
        log::Level::Warn => "warn:".yellow(),
        log::Level::Info => "info:".green(),
        log::Level::Debug => "debug:".blue(),
        log::Level::Trace => "trace:".cyan(),
      };

      writeln!(out, "{header} {message}", message = arguments.args())
    })
    .init();

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
    dix::spawn_size_diff(old_path.clone(), new_path.clone());

  let wrote = dix::write_package_diff(&mut out, &old_path, &new_path)?;

  let (size_old, size_new) = closure_size_handle.join().map_err(|_| {
    anyhow!(
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
