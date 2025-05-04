use clap::Parser;
use core::str;
use regex::Regex;
use std::{process::Command, string::String};

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
struct Package {
    name: String,
    version: String,
}

// Only there to make the compiler shut up for now.
#[derive(Debug)]
enum BlaErr {
    LolErr,
}

fn main() {
    let args = Args::parse();

    println!("Nix available: {}", check_nix_available());

    println!("Checking path one:");
    println!(
        "Path one is a system: {}",
        &args.path.join("activate").exists()
    );

    println!("Checking path two:");
    println!(
        "Path two is a system: {}",
        &args.path2.join("activate").exists()
    );

    if check_if_system(&args.path) && check_if_system(&args.path2) {
        let packages = get_packages(&args.path);
        let packages2 = get_packages(&args.path2);

        // println!("{:?}", packages);

        if let (Ok(packages), Ok(packages2)) = (packages, packages2) {
            let mut added = vec![];
            let mut removed = vec![];

            for i in &packages {
                if !packages2.contains(i) {
                    added.push(i);
                }
            }
            for i in &packages {
                if !packages2.contains(i) {
                    removed.push(i);
                }
            }

            let added_pretty: Vec<Package> =
                added.iter().map(|p| get_version(p.to_string())).collect();
            let removed_pretty: Vec<Package> =
                removed.iter().map(|p| get_version(p.to_string())).collect();

            println!("Difference between the two generations:");
            println!("Packages added: ");
            if args.paths {
                for p in added.iter() {
                    println!("A: {:?}", p);
                }
                println!();
                println!("Packages removed: ");
                for p in removed.iter() {
                    println!("R: {:?}", p);
                }
            } else {
                for p in added_pretty.iter() {
                    if !p.name.is_empty() {
                        println!("A: {} @ {}", p.name, p.version);
                    }
                }
                println!();
                println!("Packages removed: ");
                for p in removed_pretty.iter() {
                    if !p.name.is_empty() {
                        println!("R: {} @ {}", p.name, p.version);
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

fn get_version(pack: String) -> Package {
    // This is bound to break sooner or later
    let re = Regex::new(r"^/nix/store/[a-z0-9]+-([^-]+(?:-[^-]+)*)-([\d][^/]*)$").unwrap();

    // No cap frfr
    if let Some(cap) = re.captures(&pack) {
        let name = cap.get(1).unwrap().as_str().to_string();
        let version = cap.get(2).unwrap().as_str().to_string();
        return Package { name, version };
    }

    Package {
        name: "".to_string(),
        version: "".to_string(),
    }
}

fn check_nix_available() -> bool {
    let nix_available = Command::new("nix").arg("--version").output().ok();
    let nix_query_available = Command::new("nix-store").arg("--version").output().ok();

    nix_available.is_some() && nix_query_available.is_some()
}
