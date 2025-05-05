use core::str;
use std::collections::{HashMap, HashSet};
use yansi::Paint;

pub fn print_added(set: HashSet<&str>, post: &HashMap<&str, HashSet<&str>>, col_width: usize) {
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
            "[{}] {:col_width$} {} {}",
            "A:".green().bold(),
            p,
            "@".yellow().bold(),
            version_str.blue()
        );
    }
}

pub fn print_removed(set: HashSet<&str>, pre: &HashMap<&str, HashSet<&str>>, col_width: usize) {
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
            "[{}] {:col_width$} {} {}",
            "R:".red().bold(),
            p,
            "@".yellow(),
            version_str.blue()
        );
    }
}

pub fn print_changes(
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
            "[{}] {:col_width$} {} {} ~> {}",
            "C:".bold().bright_yellow(),
            p,
            "@".yellow(),
            version_str_pre.magenta(),
            version_str_post.blue()
        );
    }
}
