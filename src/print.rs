use core::str;
use std::collections::{HashMap, HashSet};
use yansi::Paint;

/// diffs two strings character by character, and returns a tuple of strings
/// colored in a way to represent the differences between the two input strings.
///
/// # Returns:
///
/// * (String, String) - The differing chars being red in the left, and green in the right one.
fn diff_versions(left: &str, right: &str) -> (String, String) {
    let mut prev = String::new();
    let mut post = String::new();

    for diff in diff::chars(left, right) {
        match diff {
            diff::Result::Both(l, _) => {
                prev.push(l);
                post.push(l);
            }
            diff::Result::Left(l) => {
                let string_to_push = format!("\x1b[31m{l}");
                prev.push_str(&string_to_push);
            }

            diff::Result::Right(r) => {
                let string_to_push = format!("\x1b[32m{r}");
                post.push_str(&string_to_push);
            }
        }
    }

    //reset
    prev.push_str("\x1b[0m");
    post.push_str("\x1b[0m");

    (prev, post)
}

/// print the packages added between two closures.
pub fn print_added(set: &HashSet<&str>, post: &HashMap<&str, HashSet<&str>>, col_width: usize) {
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
            version_str
        );
    }
}

/// print the packages removed between two closures.
pub fn print_removed(set: &HashSet<&str>, pre: &HashMap<&str, HashSet<&str>>, col_width: usize) {
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
            version_str
        );
    }
}

pub fn print_changes(
    set: &HashSet<&str>,
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
        let mut version_vec_post = ver_post.iter().copied().collect::<Vec<_>>();

        version_vec_pre.sort_unstable();
        version_vec_post.sort_unstable();

        let diffed_pre: String;
        let diffed_post: String;

        if version_vec_pre.len() == version_vec_post.len() {
            let mut diff_pre: Vec<String> = vec![];
            let mut diff_post: Vec<String> = vec![];

            for (pre, post) in version_vec_pre.iter().zip(version_vec_post.iter()) {
                let (a, b) = diff_versions(pre, post);
                diff_pre.push(a);
                diff_post.push(b);
            }
            diffed_pre = diff_pre.join(", ");
            diffed_post = diff_post.join(", ");
        } else {
            let version_str_pre = version_vec_pre.join(", ");
            let version_str_post = version_vec_post.join(", ");
            (diffed_pre, diffed_post) = diff_versions(&version_str_pre, &version_str_post);
        }

        println!(
            "[{}] {:col_width$} {} {} ~> {}",
            "C:".bold().bright_yellow(),
            p,
            "@".yellow(),
            diffed_pre,
            diffed_post
        );
    }
}
