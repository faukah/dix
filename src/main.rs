use clap::Parser;
use core::str;
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    process::Command,
    string::String,
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

#[derive(Debug, PartialEq, PartialOrd, Eq, Clone, Hash)]
struct Package<'a> {
    name: &'a str,
    version: &'a str,
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

    if check_if_system(&args.path) && check_if_system(&args.path2) {
        let packages = get_packages(&args.path);
        let packages2 = get_packages(&args.path2);

        if let (Ok(packages), Ok(packages2)) = (packages, packages2) {
            let mut pre: HashMap<String, String> = HashMap::new();
            let mut post: HashMap<String, String> = HashMap::new();

            for p in packages.iter() {
                let version = get_version(p);
                pre.insert(version.0.to_string(), version.1.to_string());
            }

            for p in packages2.iter() {
                let version = get_version(p);
                post.insert(version.0.to_string(), version.1.to_string());
            }

            let pre_keys: HashSet<String> = pre.clone().into_keys().collect();
            let post_keys: HashSet<String> = post.clone().into_keys().collect();

            let added: HashSet<String> = &post_keys - &pre_keys;
            let removed: HashSet<String> = &pre_keys - &post_keys;
            let maybe_changed: HashSet<_> = pre_keys.intersection(&post_keys).collect();

            println!("Difference between the two generations:");
            println!("Packages added: ");
            for p in added {
                let version = post.get(&p);
                if let Some(ver) = version {
                    println!("A: {} @ {}", p, ver);
                }
            }
            println!();
            println!("Packages removed: ");
            for p in removed {
                let version = pre.get(&p);
                if let Some(ver) = version {
                    println!("R: {} @ {}", p, ver);
                }
            }
            println!();
            println!("Version changes: ");
            for p in maybe_changed {
                if p.is_empty() {
                    continue;
                }
                let version_pre = pre.get(p);
                let version_post = post.get(p);

                if let (Some(ver_pre), Some(ver_post)) = (version_pre, version_post) {
                    if ver_pre != ver_post {
                        println!("C: {} @ {} -> {}", p, ver_pre, ver_post);
                    }
                }
            }
        }
    } else {
        println!("One of them is not a system!")
    }
}

fn check_if_system(path: &std::path::Path) -> bool {
    path.join("activate").exists()
}

fn get_packages(path: &std::path::Path) -> Result<Vec<String>, BlaErr> {
    let references = Command::new("nix-store")
        .arg("--query")
        .arg("--references")
        .arg(path.join("sw"))
        .output();

    if let Ok(query) = references {
        let list = str::from_utf8(&query.stdout);

        if let Ok(list) = list {
            let res: Vec<String> = list.lines().map(|s| s.to_string()).collect();
            return Ok(res);
        }
    }
    Err(BlaErr::LolErr)
}

fn get_version(pack: &str) -> (&str, &str) {
    let re = Regex::new(r"^/nix/store/[a-z0-9]+-(.+?)-([0-9].*?)$").unwrap();

    // No cap frfr
    if let Some(cap) = re.captures(pack) {
        let name = cap.get(1).unwrap().as_str();
        let version = cap.get(2).unwrap().as_str();
        return (name, version);
    }

    ("", "")
}

fn check_nix_available() -> bool {
    let nix_available = Command::new("nix").arg("--version").output().ok();
    let nix_query_available = Command::new("nix-store").arg("--version").output().ok();

    nix_available.is_some() && nix_query_available.is_some()
}
