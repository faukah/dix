use clap::Parser;
use core::str;
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    process::Command,
    string::String,
    sync::OnceLock,
    thread,
};
use thiserror::Error;
use yansi::Paint;

/// Application errors with thiserror
#[derive(Debug, Error)]
enum AppError {
    #[error("Command failed: {0}")]
    CommandFailed(String),

    #[error("Failed to decode command output: {0}")]
    CommandOutputError(#[from] std::str::Utf8Error),

    #[error("Failed to parse data: {0}")]
    ParseError(String),

    #[error("Regex error: {0}")]
    RegexError(#[from] regex::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
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

fn main() {
    let args = Args::parse();

    println!("Nix available: {}", check_nix_available());

    println!("<<< {}", args.path.to_string_lossy());
    println!(">>> {}", args.path2.to_string_lossy());

    // Handles to the threads collecting closure size information
    // We do this as early as possible because nix is slow.
    let closure_size_handles = if args.closure_size {
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
        Ok(packages) => packages,
        Err(e) => {
            eprintln!("Error getting packages from path {}: {}", args.path.display(), e);
            Vec::new()
        }
    };

    let package_list_post = match get_packages(&args.path2) {
        Ok(packages) => packages,
        Err(e) => {
            eprintln!("Error getting packages from path {}: {}", args.path2.display(), e);
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
            },
            Err(e) => {
                if cfg!(debug_assertions) {
                    eprintln!("Error parsing package version: {e}");
                }
            }
        }
    }

    for p in &package_list_post {
        match get_version(&**p) {
            Ok((name, version)) => {
                post.entry(name).or_default().insert(version);
            },
            Err(e) => {
                if cfg!(debug_assertions) {
                    eprintln!("Error parsing package version: {e}");
                }
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

    println!("Difference between the two generations:");
    println!();

    print_added(added, &post);
    println!();
    print_removed(removed, &pre);
    println!();
    print_changes(changed, &pre, &post);

    if let Some((pre_handle, post_handle)) = closure_size_handles {
        match (pre_handle.join(), post_handle.join()) {
            (Ok(Ok(pre_size)), Ok(Ok(post_size))) => {
                println!("{}", "Closure Size:".underline().bold());
                println!("Before: {pre_size} MiB");
                println!("After: {post_size} MiB");
                println!("Difference: {} MiB", post_size - pre_size);
            }
            (Ok(Err(e)), _) | (_, Ok(Err(e))) => {
                eprintln!("Error getting closure size: {e}");
            }
            _ => {
                eprintln!("Error: Failed to get closure size information due to a thread error");
            }
        }
    }
}

/// Gets the packages in a closure
fn get_packages(path: &std::path::Path) -> Result<Vec<String>> {
    // Get the nix store paths using `nix-store --query --references <path>``
    let output = Command::new("nix-store")
        .arg("--query")
        .arg("--references")
        .arg(path.join("sw"))
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::CommandFailed(format!(
            "nix-store command failed: {stderr}"
        )));
    }

    let list = str::from_utf8(&output.stdout)?;
    Ok(list.lines().map(str::to_owned).collect())
}

/// Gets the dependencies of the packages in a closure
fn get_dependencies(path: &std::path::Path) -> Result<Vec<String>> {
    // Get the nix store paths using `nix-store --query --requisites <path>``
    let output = Command::new("nix-store")
        .arg("--query")
        .arg("--requisites")
        .arg(path.join("sw"))
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::CommandFailed(format!(
            "nix-store command failed: {stderr}"
        )));
    }

    let list = str::from_utf8(&output.stdout)?;
    Ok(list.lines().map(str::to_owned).collect())
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
            return Err(AppError::ParseError(format!(
                "Failed to extract name or version from path: {path}"
            )));
        }

        return Ok((name, version));
    }

    Err(AppError::ParseError(format!(
        "Path does not match expected nix store format: {path}"
    )))
}

fn check_nix_available() -> bool {
    // Check if nix is available on the host system.
    let nix_available = Command::new("nix").arg("--version").output().ok();
    let nix_query_available = Command::new("nix-store").arg("--version").output().ok();

    nix_available.is_some() && nix_query_available.is_some()
}

fn get_closure_size(path: &std::path::Path) -> Result<i64> {
    // Run nix path-info command to get closure size
    let output = Command::new("nix")
        .arg("path-info")
        .arg("--closure-size")
        .arg(path.join("sw"))
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::CommandFailed(format!(
            "nix path-info command failed: {stderr}"
        )));
    }

    let stdout = str::from_utf8(&output.stdout)?;

    // Parse the last word in the output as an integer (in bytes)
    stdout
        .split_whitespace()
        .last()
        .ok_or_else(|| AppError::ParseError("Unexpected output format from nix path-info".into()))?
        .parse::<i64>()
        .map_err(|e| AppError::ParseError(format!("Failed to parse size value: {e}")))
        .map(|size| size / 1024 / 1024) // Convert to MiB
}

fn print_added(set: HashSet<&str>, post: &HashMap<&str, HashSet<&str>>) {
    println!("{}", "Packages added:".underline().bold());

    // Use sorted output
    let mut sorted: Vec<_> = set.iter()
        .filter_map(|p| post.get(p).map(|ver| (*p, ver)))
        .collect();

    // Sort by package name for consistent output
    sorted.sort_by(|(a, _), (b, _)| a.cmp(b));

    for (p, ver) in sorted {
        let version_str = ver.iter().copied().collect::<Vec<_>>().join(" ");
        println!(
            "{} {} {} {}",
            "[A:]".green().bold(),
            p,
            "@".yellow().bold(),
            version_str.blue()
        );
    }
}

fn print_removed(set: HashSet<&str>, pre: &HashMap<&str, HashSet<&str>>) {
    println!("{}", "Packages removed:".underline().bold());
    set.iter()
        .filter_map(|p| pre.get(p).map(|ver| (*p, ver)))
        .for_each(|(p, ver)| {
            let version_str = ver.iter().copied().collect::<Vec<_>>().join(" ");
            println!(
                "{} {} {} {}",
                "[R:]".red().bold(),
                p,
                "@".yellow(),
                version_str.blue()
            );
        });
}

fn print_changes(
    set: HashSet<&str>,
    pre: &HashMap<&str, HashSet<&str>>,
    post: &HashMap<&str, HashSet<&str>>,
) {
    println!("{}", "Version changes:".underline().bold());
    set.iter()
        .filter(|p| !p.is_empty())
        .filter_map(|p| {
            match (pre.get(p), post.get(p)) {
                (Some(ver_pre), Some(ver_post)) if ver_pre != ver_post => {
                    Some((p, ver_pre, ver_post))
                }
                _ => None,
            }
        })
        .for_each(|(p, ver_pre, ver_post)| {
            let version_str_pre = ver_pre.iter().copied().collect::<Vec<_>>().join(" ");
            let version_str_post = ver_post.iter().copied().collect::<Vec<_>>().join(", ");

            println!(
                "{} {} {} {} ~> {}",
                "[C:]".bold().bright_yellow(),
                p,
                "@".yellow(),
                version_str_pre.magenta(),
                version_str_post.blue()
            );
        });
}
