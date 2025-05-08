use clap::Parser;
use core::str;
use dixlib::print;
use dixlib::store;
use dixlib::util::PackageDiff;
use log::{debug, error};
use std::{
    collections::{HashMap, HashSet},
    thread,
};
use yansi::Paint;

#[derive(Parser, Debug)]
#[command(name = "dix")]
#[command(version = "1.0")]
#[command(about = "Diff Nix stuff", long_about = None)]
#[command(version, about, long_about = None)]
struct Args {
    path: std::path::PathBuf,
    path2: std::path::PathBuf,

    /// Print the whole store paths
    #[arg(short, long)]
    paths: bool,

    /// Print the closure size
    #[arg(long, short)]
    closure_size: bool,

    /// Verbosity level: -v for debug, -vv for trace
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Silence all output except errors
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Debug, Clone)]
struct Package<'a> {
    name: &'a str,
    versions: HashSet<&'a str>,
    /// Save if a package is a dependency of another package
    is_dep: bool,
}

impl<'a> Package<'a> {
    fn new(name: &'a str, version: &'a str, is_dep: bool) -> Self {
        let mut versions = HashSet::new();
        versions.insert(version);
        Self {
            name,
            versions,
            is_dep,
        }
    }

    fn add_version(&mut self, version: &'a str) {
        self.versions.insert(version);
    }
}

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
fn main() {
    let args = Args::parse();

    // Configure logger based on verbosity flags and environment variables
    // Respects RUST_LOG environment variable if present.
    // XXX:We can also dedicate a specific env variable for this tool, if we want to.
    let env = env_logger::Env::default().filter_or(
        "RUST_LOG",
        if args.quiet {
            "error"
        } else {
            match args.verbose {
                0 => "info",
                1 => "debug",
                _ => "trace",
            }
        },
    );

    // Build and initialize the logger
    env_logger::Builder::from_env(env)
        .format_timestamp(Some(env_logger::fmt::TimestampPrecision::Seconds))
        .init();

    // handles to the threads collecting closure size information
    // We do this as early as possible because nix is slow.
    let closure_size_handles = if args.closure_size {
        debug!("Calculating closure sizes in background");
        let path = args.path.clone();
        let path2 = args.path2.clone();
        Some((
            thread::spawn(move || store::get_closure_size(&path)),
            thread::spawn(move || store::get_closure_size(&path2)),
        ))
    } else {
        None
    };

    // Get package lists and handle potential errors
    let package_list_pre = match store::get_packages(&args.path) {
        Ok(packages) => {
            debug!("Found {} packages in first closure", packages.len());
            packages.into_iter().map(|(_, path)| path).collect()
        }
        Err(e) => {
            error!(
                "Error getting packages from path {}: {}",
                args.path.display(),
                e
            );
            eprintln!(
                "Error getting packages from path {}: {}",
                args.path.display(),
                e
            );
            Vec::new()
        }
    };

    let package_list_post = match store::get_packages(&args.path2) {
        Ok(packages) => {
            debug!("Found {} packages in second closure", packages.len());
            packages.into_iter().map(|(_, path)| path).collect()
        }
        Err(e) => {
            error!(
                "Error getting packages from path {}: {}",
                args.path2.display(),
                e
            );
            eprintln!(
                "Error getting packages from path {}: {}",
                args.path2.display(),
                e
            );
            Vec::new()
        }
    };

    let PackageDiff {
        pkg_to_versions_pre: pre,
        pkg_to_versions_post: post,
        pre_keys: _,
        post_keys: _,
        added,
        removed,
        changed,
    } = PackageDiff::new(&package_list_pre, &package_list_post);

    debug!("Added packages: {}", added.len());
    debug!("Removed packages: {}", removed.len());
    debug!(
        "Changed packages: {}",
        changed
            .iter()
            .filter(|p| !p.is_empty()
                && match (pre.get(*p), post.get(*p)) {
                    (Some(ver_pre), Some(ver_post)) => ver_pre != ver_post,
                    _ => false,
                })
            .count()
    );

    println!("Difference between the two generations:");
    println!();

    let width_changes = changed
        .iter()
        .filter(|&&p| match (pre.get(p), post.get(p)) {
            (Some(version_pre), Some(version_post)) => version_pre != version_post,
            _ => false,
        });

    let col_width = added
        .iter()
        .chain(removed.iter())
        .chain(width_changes)
        .map(|p| p.len())
        .max()
        .unwrap_or_default();

    println!("<<< {}", args.path.to_string_lossy());
    println!(">>> {}", args.path2.to_string_lossy());
    print::print_added(&added, &post, col_width);
    print::print_removed(&removed, &pre, col_width);
    print::print_changes(&changed, &pre, &post, col_width);

    if let Some((pre_handle, post_handle)) = closure_size_handles {
        match (pre_handle.join(), post_handle.join()) {
            (Ok(Ok(pre_size)), Ok(Ok(post_size))) => {
                let pre_size = pre_size / 1024 / 1024;
                let post_size = post_size / 1024 / 1024;
                debug!("Pre closure size: {pre_size} MiB");
                debug!("Post closure size: {post_size} MiB");

                println!("{}", "Closure Size:".underline().bold());
                println!("Before: {pre_size} MiB");
                println!("After: {post_size} MiB");
                println!("Difference: {} MiB", post_size - pre_size);
            }
            (Ok(Err(e)), _) | (_, Ok(Err(e))) => {
                error!("Error getting closure size: {e}");
                eprintln!("Error getting closure size: {e}");
            }
            _ => {
                error!("Failed to get closure size information due to a thread error");
                eprintln!("Error: Failed to get closure size information due to a thread error");
            }
        }
    }
}
