use clap::Parser;
use colored::Colorize;
use core::str;
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    process::Command,
    string::{String, ToString},
};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    path: std::path::PathBuf,
    path2: std::path::PathBuf,

    /// Print the whole store paths
    #[arg(short, long)]
    paths: bool,
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
        let pre_keys: HashSet<String> = pre.clone().into_keys().collect();
        let post_keys: HashSet<String> = post.clone().into_keys().collect();
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
            let version_pre = pre.get(p);
            let version_post = post.get(p);

            if let (Some(ver_pre), Some(ver_post)) = (version_pre, version_post) {
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
