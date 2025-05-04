use clap::Parser;
use core::str;
use env_logger;
use log::{debug, error, info, warn};
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    process::Command,
    string::{String, ToString},
    sync::OnceLock,
    thread,
};
use thiserror::Error;
use yansi::Paint;

/// Application errors with thiserror
#[derive(Debug, Error)]
enum AppError {
    #[error("Command failed: {command} {args:?} - {message}")]
    CommandFailed {
        command: String,
        args: Vec<String>,
        message: String,
    },

    #[error("Failed to decode command output from {context}: {source}")]
    CommandOutputError {
        source: std::str::Utf8Error,
        context: String,
    },

    #[error("Failed to parse data in {context}: {message}")]
    ParseError {
        message: String,
        context: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("Regex error in {context}: {source}")]
    RegexError {
        source: regex::Error,
        context: String,
    },

    #[error("IO error in {context}: {source}")]
    IoError {
        source: std::io::Error,
        context: String,
    },

    #[error("Nix store error: {message} for path: {store_path}")]
    NixStoreError { message: String, store_path: String },
}

// Implement From traits to support the ? operator
impl From<std::io::Error> for AppError {
    fn from(source: std::io::Error) -> Self {
        Self::IoError {
            source,
            context: "unknown context".into(),
        }
    }
}

impl From<std::str::Utf8Error> for AppError {
    fn from(source: std::str::Utf8Error) -> Self {
        Self::CommandOutputError {
            source,
            context: "command output".into(),
        }
    }
}

impl From<regex::Error> for AppError {
    fn from(source: regex::Error) -> Self {
        Self::RegexError {
            source,
            context: "regex operation".into(),
        }
    }
}

impl AppError {
    /// Create a command failure error with context
    pub fn command_failed<S: Into<String>>(command: S, args: &[&str], message: S) -> Self {
        Self::CommandFailed {
            command: command.into(),
            args: args.iter().map(|&s| s.to_string()).collect(),
            message: message.into(),
        }
    }

    /// Create a parse error with context
    pub fn parse_error<S: Into<String>, C: Into<String>>(
        message: S,
        context: C,
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self::ParseError {
            message: message.into(),
            context: context.into(),
            source,
        }
    }

    /// Create an IO error with context
    pub fn io_error<C: Into<String>>(source: std::io::Error, context: C) -> Self {
        Self::IoError {
            source,
            context: context.into(),
        }
    }

    /// Create a regex error with context
    pub fn regex_error<C: Into<String>>(source: regex::Error, context: C) -> Self {
        Self::RegexError {
            source,
            context: context.into(),
        }
    }

    /// Create a command output error with context
    pub fn command_output_error<C: Into<String>>(source: std::str::Utf8Error, context: C) -> Self {
        Self::CommandOutputError {
            source,
            context: context.into(),
        }
    }

    /// Create a Nix store error
    pub fn nix_store_error<M: Into<String>, P: Into<String>>(message: M, store_path: P) -> Self {
        Self::NixStoreError {
            message: message.into(),
            store_path: store_path.into(),
        }
    }
}

// Use type alias for Result with our custom error type
type Result<T> = std::result::Result<T, AppError>;

#[derive(Parser, Debug)]
#[command(name = "Nix not Python diff tool")]
#[command(version = "1.0")]
#[command(about = "Diff two different system closures", long_about = None)]
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

    debug!("Nix available: {}", check_nix_available()); // XXX: is this supposed to be user-facing?
    println!("<<< {}", args.path.to_string_lossy());
    println!(">>> {}", args.path2.to_string_lossy());

    // handles to the threads collecting closure size information
    // We do this as early as possible because nix is slow.
    let closure_size_handles = if args.closure_size {
        debug!("Calculating closure sizes in background");
        let path = args.path.clone();
        let path2 = args.path2.clone();
        Some((
            thread::spawn(move || get_closure_size(&path)),
            thread::spawn(move || get_closure_size(&path2)),
        ))
    } else {
        None
    };

    // Get package lists and handle potential errors
    let package_list_pre = match get_packages(&args.path) {
        Ok(packages) => {
            debug!("Found {} packages in first closure", packages.len());
            packages
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

    let package_list_post = match get_packages(&args.path2) {
        Ok(packages) => {
            debug!("Found {} packages in second closure", packages.len());
            packages
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

    // Map from packages of the first closure to their version
    let mut pre = HashMap::<&str, HashSet<&str>>::new();
    let mut post = HashMap::<&str, HashSet<&str>>::new();

    for p in &package_list_pre {
        match get_version(&**p) {
            Ok((name, version)) => {
                pre.entry(name).or_default().insert(version);
            }
            Err(e) => {
                debug!("Error parsing package version: {}", e);
            }
        }
    }

    for p in &package_list_post {
        match get_version(&**p) {
            Ok((name, version)) => {
                post.entry(name).or_default().insert(version);
            }
            Err(e) => {
                debug!("Error parsing package version: {}", e);
            }
        }
    }

    // Compare the package names of both versions
    let pre_keys: HashSet<&str> = pre.keys().copied().collect();
    let post_keys: HashSet<&str> = post.keys().copied().collect();

    // Difference gives us added and removed packages
    let added: HashSet<&str> = &post_keys - &pre_keys;
    let removed: HashSet<&str> = &pre_keys - &post_keys;
    // Get the intersection of the package names for version changes
    let changed: HashSet<&str> = &pre_keys & &post_keys;

    debug!("Added packages: {}", added.len());
    debug!("Removed packages: {}", removed.len());
    debug!(
        "Changed packages: {}",
        changed
            .iter()
            .filter(|p| !p.is_empty())
            .filter_map(|p| match (pre.get(p), post.get(p)) {
                (Some(ver_pre), Some(ver_post)) if ver_pre != ver_post => Some(p),
                _ => None,
            })
            .count()
    );

    println!("Difference between the two generations:");
    println!();

    let mut width_changes = changed
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

    print_added(added, &post, col_width);
    println!();
    print_removed(removed, &pre, col_width);
    println!();
    print_changes(changed, &pre, &post, col_width);

    if let Some((pre_handle, post_handle)) = closure_size_handles {
        match (pre_handle.join(), post_handle.join()) {
            (Ok(Ok(pre_size)), Ok(Ok(post_size))) => {
                debug!("Pre closure size: {} MiB", pre_size);
                debug!("Post closure size: {} MiB", post_size);

                println!("{}", "Closure Size:".underline().bold());
                println!("Before: {pre_size} MiB");
                println!("After: {post_size} MiB");
                println!("Difference: {} MiB", post_size - pre_size);
            }
            (Ok(Err(e)), _) | (_, Ok(Err(e))) => {
                error!("Error getting closure size: {}", e);
                eprintln!("Error getting closure size: {e}");
            }
            _ => {
                error!("Failed to get closure size information due to a thread error");
                eprintln!("Error: Failed to get closure size information due to a thread error");
            }
        }
    }
}

/// Gets the packages in a closure
fn get_packages(path: &std::path::Path) -> Result<Vec<String>> {
    debug!("Getting packages from path: {}", path.display());

    // Get the nix store paths using `nix-store --query --references <path>``
    let output = Command::new("nix-store")
        .arg("--query")
        .arg("--references")
        .arg(path.join("sw"))
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("nix-store command failed: {}", stderr);
        return Err(AppError::CommandFailed {
            command: "nix-store".to_string(),
            args: vec![
                "--query".to_string(),
                "--references".to_string(),
                path.join("sw").to_string_lossy().to_string(),
            ],
            message: stderr.to_string(),
        });
    }

    let list = str::from_utf8(&output.stdout)?;
    let packages: Vec<String> = list.lines().map(str::to_owned).collect();
    debug!("Found {} packages", packages.len());
    Ok(packages)
}

/// Gets the dependencies of the packages in a closure
fn get_dependencies(path: &std::path::Path) -> Result<Vec<String>> {
    debug!("Getting dependencies from path: {}", path.display());

    // Get the nix store paths using `nix-store --query --requisites <path>`
    let output = Command::new("nix-store")
        .arg("--query")
        .arg("--requisites")
        .arg(path.join("sw"))
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("nix-store command failed: {}", stderr);
        return Err(AppError::CommandFailed {
            command: "nix-store".to_string(),
            args: vec![
                "--query".to_string(),
                "--requisites".to_string(),
                path.join("sw").to_string_lossy().to_string(),
            ],
            message: stderr.to_string(),
        });
    }

    let list = str::from_utf8(&output.stdout)?;
    let dependencies: Vec<String> = list.lines().map(str::to_owned).collect();
    debug!("Found {} dependencies", dependencies.len());
    Ok(dependencies)
}

// Returns a reference to the compiled regex pattern.
// The regex is compiled only once.
fn store_path_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^/nix/store/[a-z0-9]+-(.+?)-([0-9].*?)$")
            .expect("Failed to compile regex pattern for nix store paths")
    })
}

// Parse the nix store path to extract package name and version
fn get_version<'a>(pack: impl Into<&'a str>) -> Result<(&'a str, &'a str)> {
    let path = pack.into();

    // Match the regex against the input
    if let Some(cap) = store_path_regex().captures(path) {
        // Handle potential missing captures safely
        let name = cap.get(1).map_or("", |m| m.as_str());
        let version = cap.get(2).map_or("", |m| m.as_str());

        if name.is_empty() || version.is_empty() {
            return Err(AppError::ParseError {
                message: format!("Failed to extract name or version from path: {path}"),
                context: "get_version".to_string(),
                source: None,
            });
        }

        return Ok((name, version));
    }

    Err(AppError::ParseError {
        message: format!("Path does not match expected nix store format: {path}"),
        context: "get_version".to_string(),
        source: None,
    })
}

fn check_nix_available() -> bool {
    // Check if nix is available on the host system.
    debug!("Checking nix command availability");
    let nix_available = Command::new("nix").arg("--version").output().ok();
    let nix_query_available = Command::new("nix-store").arg("--version").output().ok();

    let result = nix_available.is_some() && nix_query_available.is_some();
    if !result {
        warn!("Nix commands not available, functionality may be limited");
    }

    result
}

fn get_closure_size(path: &std::path::Path) -> Result<i64> {
    debug!("Calculating closure size for path: {}", path.display());

    // Run nix path-info command to get closure size
    let output = Command::new("nix")
        .arg("path-info")
        .arg("--closure-size")
        .arg(path.join("sw"))
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("nix path-info command failed: {}", stderr);
        return Err(AppError::CommandFailed {
            command: "nix".to_string(),
            args: vec![
                "path-info".to_string(),
                "--closure-size".to_string(),
                path.join("sw").to_string_lossy().to_string(),
            ],
            message: stderr.to_string(),
        });
    }

    let stdout = str::from_utf8(&output.stdout)?;

    // Parse the last word in the output as an integer (in bytes)
    let size = stdout
        .split_whitespace()
        .last()
        .ok_or_else(|| {
            let err = "Unexpected output format from nix path-info";
            error!("{}", err);
            AppError::ParseError {
                message: err.into(),
                context: "get_closure_size".to_string(),
                source: None,
            }
        })?
        .parse::<i64>()
        .map_err(|e| {
            let err = format!("Failed to parse size value: {e}");
            error!("{}", err);
            AppError::ParseError {
                message: err,
                context: "get_closure_size".to_string(),
                source: None,
            }
        })?;

    // Convert to MiB
    let size_mib = size / 1024 / 1024;
    debug!("Closure size for {}: {} MiB", path.display(), size_mib);
    Ok(size_mib)
}

fn print_added(set: HashSet<&str>, post: &HashMap<&str, HashSet<&str>>, col_width: usize) {
    println!("{}", "Packages added:".underline().bold());

    // Use sorted outpu
    let mut sorted: Vec<_> = set
        .iter()
        .filter_map(|p| post.get(p).map(|ver| (*p, ver)))
        .collect();

    // Sort by package name for consistent output
    sorted.sort_by(|(a, _), (b, _)| a.cmp(b));

    for (p, ver) in sorted {
        let mut version_vec = ver.iter().copied().collect::<Vec<_>>();
        version_vec.sort_unstable();
        let version_str = version_vec.join(", ");
        println!(
            "{} {:col_width$} {} {}",
            "[A:]".green().bold(),
            p,
            "@".yellow().bold(),
            version_str.blue()
        );
    }
}

fn print_removed(set: HashSet<&str>, pre: &HashMap<&str, HashSet<&str>>, col_width: usize) {
    println!("{}", "Packages removed:".underline().bold());

    // Use sorted output for more predictable and readable results
    let mut sorted: Vec<_> = set
        .iter()
        .filter_map(|p| pre.get(p).map(|ver| (*p, ver)))
        .collect();

    // Sort by package name for consistent output
    sorted.sort_by(|(a, _), (b, _)| a.cmp(b));

    for (p, ver) in sorted {
        let mut version_vec = ver.iter().copied().collect::<Vec<_>>();
        version_vec.sort_unstable();
        let version_str = version_vec.join(", ");
        println!(
            "{} {:col_width$} {} {}",
            "[R:]".red().bold(),
            p,
            "@".yellow(),
            version_str.blue()
        );
    }
}

fn print_changes(
    set: HashSet<&str>,
    pre: &HashMap<&str, HashSet<&str>>,
    post: &HashMap<&str, HashSet<&str>>,
    col_width: usize,
) {
    println!("{}", "Version changes:".underline().bold());

    // Use sorted output for more predictable and readable results
    let mut changes = Vec::new();

    for p in set.iter().filter(|p| !p.is_empty()) {
        if let (Some(ver_pre), Some(ver_post)) = (pre.get(p), post.get(p)) {
            if ver_pre != ver_post {
                changes.push((*p, ver_pre, ver_post));
            }
        }
    }

    // Sort by package name for consistent output
    changes.sort_by(|(a, _, _), (b, _, _)| a.cmp(b));

    for (p, ver_pre, ver_post) in changes {
        let mut version_vec_pre = ver_pre.iter().copied().collect::<Vec<_>>();
        version_vec_pre.sort_unstable();
        let version_str_pre = version_vec_pre.join(", ");
        let mut version_vec_post = ver_post.iter().copied().collect::<Vec<_>>();

        version_vec_post.sort_unstable();
        let version_str_post = version_vec_post.join(", ");

        println!(
            "{} {:col_width$} {} {} ~> {}",
            "[C:]".bold().bright_yellow(),
            p,
            "@".yellow(),
            version_str_pre.magenta(),
            version_str_post.blue()
        );
    }
}
