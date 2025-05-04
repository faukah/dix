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

// Only there to make the compiler shut up for now.
#[derive(Debug)]
enum BlaErr {
    LolErr,
}

fn main() {
    let args = Args::parse();

    println!("Nix available: {}", check_nix_available());

    println!("<<< {}", args.path.to_string_lossy());
    println!(">>> {}", args.path2.to_string_lossy());

    // handles to the threads collecting closure size information
    let mut csize_pre_handle = None;
    let mut csize_post_handle = None;

    // get closure size in the background to increase performance
    if args.closure_size {
        let (p1, p2) = (args.path.clone(), args.path2.clone());
        csize_pre_handle = Some(thread::spawn(move || get_closure_size(&p1)));
        csize_post_handle = Some(thread::spawn(move || get_closure_size(&p2)));
    }

    let packages = get_packages(&args.path);
    let packages2 = get_packages(&args.path2);

    if let (Ok(packages), Ok(packages2)) = (packages, packages2) {
        // Map from packages of the first closure to their version
        let pre: HashMap<String, String> = packages
            .into_iter()
            .map(|p| {
                let (name, version) = get_version(&*p);
                (name.to_string(), version.to_string())
            })
            .collect();

        let post: HashMap<String, String> = packages2
            .into_iter()
            .map(|p| {
                let (name, version) = get_version(&*p);
                (name.to_string(), version.to_string())
            })
            .collect();

        // Compare the package names of both versions
        let pre_keys: HashSet<String> = pre.keys().map(|k| k.clone()).collect();
        let post_keys: HashSet<String> = post.keys().map(|k| k.clone()).collect();
        // get the intersection of the package names for version changes
        let maybe_changed: HashSet<_> = pre_keys.intersection(&post_keys).collect();

        // difference gives us added and removed packages
        let added: HashSet<String> = &post_keys - &pre_keys;
        let removed: HashSet<String> = &pre_keys - &post_keys;

        println!("Difference between the two generations:");
        println!("{}", "Packages added:".underline().bold());
        for p in added {
            let version = post.get(&p);
            if let Some(ver) = version {
                println!(
                    "{} {} {} {}",
                    "[A:]".green().bold(),
                    p,
                    "@".yellow().bold(),
                    ver.cyan()
                );
            }
        }
        println!();
        println!("{}", "Packages removed:".underline().bold());
        for p in removed {
            let version = pre.get(&p);
            if let Some(ver) = version {
                println!(
                    "{} {} {} {}",
                    "[R:]".red().bold(),
                    p,
                    "@".yellow(),
                    ver.cyan()
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

            if ver_pre != ver_post {
                // println!("C: {p} @ {ver_pre} -> {ver_post}");
                println!(
                    "{} {} {} {} {} {}",
                    "[C:]".purple().bold(),
                    p,
                    "@".yellow(),
                    ver_pre.yellow(),
                    "~>".purple(),
                    ver_post.cyan()
                );
            }
        }
        if args.closure_size {
            let closure_size_pre = csize_pre_handle.unwrap().join().unwrap() as i64;
            let closure_size_post = csize_post_handle.unwrap().join().unwrap() as i64;

            println!("{}", "Closure Size:".underline().bold());

            println!("Before: {} MiB", closure_size_pre);
            println!("After: {} MiB", closure_size_post);
            println!("Difference: {} MiB", closure_size_post - closure_size_pre);
        }
    }
}

fn get_packages(path: &std::path::Path) -> Result<Vec<String>, BlaErr> {
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
            return Ok(res);
        }
    }
    Err(BlaErr::LolErr)
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

fn get_closure_size(path: &std::path::Path) -> u64 {
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
                .parse::<u64>()
                .ok()
        })
        .map(|size| size / 1024 / 1024)
        .unwrap_or(0)
}
