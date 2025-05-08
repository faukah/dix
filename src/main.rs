use std::{
  fmt::Write as _,
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
};
use clap::Parser as _;
use dix::{
  diff,
  store,
};
use yansi::Paint as _;

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
  let _closure_size_handle = {
    log::debug!("calculating closure sizes in background");

    let mut connection = store::connect()?;

    let old_path = old_path.clone();
    let new_path = new_path.clone();

    thread::spawn(move || {
      Ok::<_, Error>((
        connection.query_closure_size(&old_path)?,
        connection.query_closure_size(&new_path)?,
      ))
    })
  };

  let mut connection = store::connect()?;

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

  let mut out = io::stdout();

  #[expect(clippy::pattern_type_mismatch)]
  diff(
    &mut out,
    paths_old.iter().map(|(_, path)| path),
    paths_new.iter().map(|(_, path)| path),
  )?;
  // let PackageDiff {
  //   pkg_to_versions_pre: pre,
  //   pkg_to_versions_post: post,
  //   pre_keys: _,
  //   post_keys: _,
  //   added,
  //   removed,
  //   changed,
  // } = PackageDiff::new(&packages_old, &packages_after);

  // log::debug!("Added packages: {}", added.len());
  // log::debug!("Removed packages: {}", removed.len());
  // log::debug!(
  //   "Changed packages: {}",
  //   changed
  //     .iter()
  //     .filter(|p| {
  //       !p.is_empty()
  //         && match (pre.get(*p), post.get(*p)) {
  //           (Some(ver_pre), Some(ver_post)) => ver_pre != ver_post,
  //           _ => false,
  //         }
  //     })
  //     .count()
  // );

  // println!("Difference between the two generations:");
  // println!();

  // let width_changes = changed.iter().filter(|&&p| {
  //   match (pre.get(p), post.get(p)) {
  //     (Some(version_pre), Some(version_post)) => version_pre != version_post,
  //     _ => false,
  //   }
  // });

  // let col_width = added
  //   .iter()
  //   .chain(removed.iter())
  //   .chain(width_changes)
  //   .map(|p| p.len())
  //   .max()
  //   .unwrap_or_default();

  // println!("<<< {}", cli.path.to_string_lossy());
  // println!(">>> {}", cli.path2.to_string_lossy());
  // print::print_added(&added, &post, col_width);
  // print::print_removed(&removed, &pre, col_width);
  // print::print_changes(&changed, &pre, &post, col_width);

  // if let Some((pre_handle, post_handle)) = closure_size_handles {
  //   match (pre_handle.join(), post_handle.join()) {
  //     (Ok(Ok(pre_size)), Ok(Ok(post_size))) => {
  //       let pre_size = pre_size / 1024 / 1024;
  //       let post_size = post_size / 1024 / 1024;
  //       log::debug!("Pre closure size: {pre_size} MiB");
  //       log::debug!("Post closure size: {post_size} MiB");

  //       println!("{}", "Closure Size:".underline().bold());
  //       println!("Before: {pre_size} MiB");
  //       println!("After: {post_size} MiB");
  //       println!("Difference: {} MiB", post_size - pre_size);
  //     },
  //     (Ok(Err(e)), _) | (_, Ok(Err(e))) => {
  //       log::error!("Error getting closure size: {e}");
  //       eprintln!("Error getting closure size: {e}");
  //     },
  //     _ => {
  //       log::error!(
  //         "Failed to get closure size information due to a thread error"
  //       );
  //       eprintln!(
  //         "Error: Failed to get closure size information due to a thread
  // error"       );
  //     },
  //   }
  // }

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

    let _ = if let Some((_, first)) = chars.next()
      && let Some((second_start, second)) = chars.next()
      && second.is_lowercase()
    {
      writeln!(
        err,
        "{first_lowercase}{rest}",
        first_lowercase = first.to_lowercase(),
        rest = &message[second_start..],
      )
    } else {
      writeln!(err, "{message}")
    };
  }

  process::exit(1);
}
