use clap::Parser;
use colored::Colorize;
use core::str;
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    process::Command,
    string::{String, ToString},
    thread,
};

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
    // get the intersection of the package names for version changes
    let maybe_changed: HashSet<_> = pre_keys.intersection(&post_keys).collect();

    // difference gives us added and removed packages
    let added: HashSet<&str> = &post_keys - &pre_keys;
    let removed: HashSet<&str> = &pre_keys - &post_keys;

    println!("Difference between the two generations:");
    println!("{}", "Packages added:".underline().bold());
    for p in added {
        let versions = post.get(&p);
        if let Some(ver) = versions {
            let version_str = ver.iter().copied().collect::<Vec<_>>().join(" ").cyan();
            println!(
                "{} {} {} {}",
                "[A:]".green().bold(),
                p,
                "@".yellow().bold(),
                version_str
            );
        }
    }
    println!();
    println!("{}", "Packages removed:".underline().bold());
    for p in removed {
        let version = pre.get(&p);
        if let Some(ver) = version {
            let version_str = ver.iter().copied().collect::<Vec<_>>().join(" ").cyan();
            println!(
                "{} {} {} {}",
                "[R:]".red().bold(),
                p,
                "@".yellow(),
                version_str
            );
        }
    }
    println!();
    println!("{}", "Version changes:".underline().bold());
    for p in maybe_changed {
        if p.is_empty() {
            continue;
        }

        // can not fail since maybe_changed is the union of the keys of pre and post
        let ver_pre = pre.get(p).unwrap();
        let ver_post = post.get(p).unwrap();
        let version_str_pre = ver_pre.iter().copied().collect::<Vec<_>>().join(" ").cyan();
        let version_str_post = ver_post
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
            .cyan();

        if ver_pre != ver_post {
            // println!("C: {p} @ {ver_pre} -> {ver_post}");
            println!(
                "{} {} {} {} {} {}",
                "[C:]".purple().bold(),
                p,
                "@".yellow(),
                version_str_pre.yellow(),
                "~>".purple(),
                version_str_post.cyan()
            );
        }
    }
    if let Some((pre_handle, post_handle)) = closure_size_handles {
        let pre_size = pre_handle.join().unwrap();
        let post_size = post_handle.join().unwrap();

        println!("{}", "Closure Size:".underline().bold());
        println!("Before: {pre_size} MiB");
        println!("After: {post_size} MiB");
        println!("Difference: {} MiB", post_size - pre_size);
    }
}

// gets the packages in a closure
fn get_packages(path: &std::path::Path) -> Vec<String> {
    // get the nix store paths using nix-store --query --references <path>
    let references = Command::new("nix-store")
        .arg("--query")
        .arg("--references")
        .arg(path.join("sw"))
        .output();

    if let Ok(query) = references {
        let list = str::from_utf8(&query.stdout);

        if let Ok(list) = list {
            let res: Vec<String> = list.lines().map(ToString::to_string).collect();
            return res;
        }
    }
    Vec::new()
}

// gets the dependencies of the packages in a closure
fn get_dependencies(path: &std::path::Path) -> Vec<String> {
    // get the nix store paths using nix-store --query --references <path>
    let references = Command::new("nix-store")
        .arg("--query")
        .arg("--requisites")
        .arg(path.join("sw"))
        .output();

    if let Ok(query) = references {
        let list = str::from_utf8(&query.stdout);

        if let Ok(list) = list {
            let res: Vec<String> = list.lines().map(ToString::to_string).collect();
            return res;
        }
    }
    Vec::new()
}

fn get_version<'a>(pack: impl Into<&'a str>) -> (&'a str, &'a str) {
    // funny regex to split a nix store path into its name and its version.
    let re = Regex::new(r"^/nix/store/[a-z0-9]+-(.+?)-([0-9].*?)$").unwrap();

    // No cap frfr
    if let Some(cap) = re.captures(pack.into()) {
        let name = cap.get(1).unwrap().as_str();
        let version = cap.get(2).unwrap().as_str();
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
    Command::new("nix")
        .arg("path-info")
        .arg("--closure-size")
        .arg(path.join("sw"))
        .output()
        .ok()
        .and_then(|output| {
            str::from_utf8(&output.stdout)
                .ok()?
                .split_whitespace()
                .last()?
                .parse::<i64>()
                .ok()
        })
        .map_or(0, |size| size / 1024 / 1024)
}
