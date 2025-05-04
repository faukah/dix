use clap::Parser;
use core::str;
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    process::Command,
    string::{String, ToString},
    sync::OnceLock,
    thread,
};
use yansi::Paint;

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

struct Package<'a> {
    name: &'a str,
    versions: HashSet<&'a str>,
    /// Save if a package is a dependency of another package
    is_dep: bool,
}

fn main() {
    let args = Args::parse();

    println!("Nix available: {}", check_nix_available());

    println!("<<< {}", args.path.to_string_lossy());
    println!(">>> {}", args.path2.to_string_lossy());

    // handles to the threads collecting closure size information
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

    let package_list_pre = get_packages(&args.path);
    let package_list_post = get_packages(&args.path2);

    // Map from packages of the first closure to their version
    let mut pre = HashMap::<&str, HashSet<&str>>::new();
    let mut post = HashMap::<&str, HashSet<&str>>::new();

    for p in &package_list_pre {
        let (name, version) = get_version(&**p);
        pre.entry(name).or_default().insert(version);
    }
    for p in &package_list_post {
        let (name, version) = get_version(&**p);
        post.entry(name).or_default().insert(version);
    }

    // Compare the package names of both versions
    let pre_keys: HashSet<&str> = pre.keys().copied().collect();
    let post_keys: HashSet<&str> = post.keys().copied().collect();

    // difference gives us added and removed packages
    let added: HashSet<&str> = &post_keys - &pre_keys;
    let removed: HashSet<&str> = &pre_keys - &post_keys;
    // get the intersection of the package names for version changes
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
            (Ok(pre_size), Ok(post_size)) => {
                println!("{}", "Closure Size:".underline().bold());
                println!("Before: {pre_size} MiB");
                println!("After: {post_size} MiB");
                println!("Difference: {} MiB", post_size - pre_size);
            }
            _ => {
                eprintln!("Error: Failed to get closure size information due to a thread error");
            }
        }
    }
}

// gets the packages in a closure
fn get_packages(path: &std::path::Path) -> Vec<String> {
    // get the nix store paths using nix-store --query --references <path>
    let output = Command::new("nix-store")
        .arg("--query")
        .arg("--references")
        .arg(path.join("sw"))
        .output();

    match output {
        Ok(query) => {
            match str::from_utf8(&query.stdout) {
                Ok(list) => list.lines().map(ToString::to_string).collect(),
                Err(e) => {
                    eprintln!("Error decoding command output: {}", e);
                    Vec::new()
                }
            }
        }
        Err(e) => {
            eprintln!("Error executing nix-store command: {}", e);
            Vec::new()
        }
    }
}

// gets the dependencies of the packages in a closure
fn get_dependencies(path: &std::path::Path) -> Vec<String> {
    // get the nix store paths using nix-store --query --references <path>
    let output = Command::new("nix-store")
        .arg("--query")
        .arg("--requisites")
        .arg(path.join("sw"))
        .output();

    match output {
        Ok(query) => {
            match str::from_utf8(&query.stdout) {
                Ok(list) => list.lines().map(ToString::to_string).collect(),
                Err(e) => {
                    eprintln!("Error decoding command output: {}", e);
                    Vec::new()
                }
            }
        }
        Err(e) => {
            eprintln!("Error executing nix-store command: {}", e);
            Vec::new()
        }
    }
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

fn get_version<'a>(pack: impl Into<&'a str>) -> (&'a str, &'a str) {
    // funny regex to split a nix store path into its name and its version.
    let path = pack.into();
    
    // Match the regex against the input
    if let Some(cap) = store_path_regex().captures(path) {
        // Handle potential missing captures safely
        let name = cap.get(1).map_or("", |m| m.as_str());
        let version = cap.get(2).map_or("", |m| m.as_str());
        return (name, version);
    }

    ("", "")
}

fn check_nix_available() -> bool {
    // Check if nix is available on the host system.
    let nix_available = Command::new("nix").arg("--version").output().ok();
    let nix_query_available = Command::new("nix-store").arg("--version").output().ok();

    nix_available.is_some() && nix_query_available.is_some()
}

fn get_closure_size(path: &std::path::Path) -> i64 {
    // Run nix path-info command to get closure size
    match Command::new("nix")
        .arg("path-info")
        .arg("--closure-size")
        .arg(path.join("sw"))
        .output()
    {
        Ok(output) if output.status.success() => {
            // Parse command output to extract the size
            match str::from_utf8(&output.stdout) {
                Ok(stdout) => {
                    // Parse the last word in the output as an integer
                    stdout
                        .split_whitespace()
                        .last()
                        .and_then(|s| s.parse::<i64>().ok())
                        .map_or_else(
                            || {
                                eprintln!("Failed to parse closure size from output: {}", stdout);
                                0
                            },
                            |size| size / 1024 / 1024, // Convert to MiB
                        )
                }
                Err(e) => {
                    eprintln!("Error decoding command output: {}", e);
                    0
                }
            }
        }
        Ok(output) => {
            // Command ran but returned an error
            match str::from_utf8(&output.stderr) {
                Ok(stderr) if !stderr.is_empty() => {
                    eprintln!("nix path-info command failed: {}", stderr);
                }
                _ => {
                    eprintln!("nix path-info command failed with status: {}", output.status);
                }
            }
            0
        }
        Err(e) => {
            // Command failed to run
            eprintln!("Failed to execute nix path-info command: {}", e);
            0
        }
    }
}

fn print_added(set: HashSet<&str>, post: &HashMap<&str, HashSet<&str>>) {
    println!("{}", "Packages added:".underline().bold());
    for p in set {
        let posts = post.get(p);
        if let Some(ver) = posts {
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
}
fn print_removed(set: HashSet<&str>, pre: &HashMap<&str, HashSet<&str>>) {
    println!("{}", "Packages removed:".underline().bold());
    for p in set {
        let pre = pre.get(p);
        if let Some(ver) = pre {
            let version_str = ver.iter().copied().collect::<Vec<_>>().join(" ");
            println!(
                "{} {} {} {}",
                "[R:]".red().bold(),
                p,
                "@".yellow(),
                version_str.blue()
            );
        }
    }
}
fn print_changes(
    set: HashSet<&str>,
    pre: &HashMap<&str, HashSet<&str>>,
    post: &HashMap<&str, HashSet<&str>>,
) {
    println!("{}", "Version changes:".underline().bold());
    for p in set {
        if p.is_empty() {
            continue;
        }

        // We should handle the case where the package might not exist in one of the maps
        if let (Some(ver_pre), Some(ver_post)) = (pre.get(p), post.get(p)) {
            let version_str_pre = ver_pre.iter().copied().collect::<Vec<_>>().join(" ");
            let version_str_post = ver_post.iter().copied().collect::<Vec<_>>().join(", ");

            if ver_pre != ver_post {
                println!(
                    "{} {} {} {} ~> {}",
                    "[C:]".bold().bright_yellow(),
                    p,
                    "@".yellow(),
                    version_str_pre.magenta(),
                    version_str_post.blue()
                );
            }
        }
    }
}
