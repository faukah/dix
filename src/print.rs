use core::str;
use std::{
  collections::{
    HashMap,
    HashSet,
  },
  string::ToString,
  sync::OnceLock,
};

use regex::Regex;
use yansi::Paint;

/// diffs two strings character by character, and returns a tuple of strings
/// colored in a way to represent the differences between the two input strings.
///
/// # Returns:
///
/// * (String, String) - The differing chars being red in the left, and green in
///   the right one.
fn diff_versions(left: &str, right: &str) -> (String, String) {
  let mut prev = "\x1b[33m".to_string();
  let mut post = "\x1b[33m".to_string();

  // We only have to filter the left once, since we stop if the left one is
  // empty. We do this to display things like -man, -dev properly.
  let matches = name_regex().captures(left);
  let mut suffix = String::new();

  if let Some(m) = matches {
    let tmp = m.get(0).map_or("", |m| m.as_str());
    suffix.push_str(tmp);
  }
  // string without the suffix
  let filtered_left = &left[..left.len() - suffix.len()];
  let filtered_right = &right[..right.len() - suffix.len()];

  for diff in diff::chars(filtered_left, filtered_right) {
    match diff {
      diff::Result::Both(l, _) => {
        let string_to_push = format!("{l}");
        prev.push_str(&string_to_push);
        post.push_str(&string_to_push);
      },
      diff::Result::Left(l) => {
        let string_to_push = format!("\x1b[1;91m{l}");
        prev.push_str(&string_to_push);
      },

      diff::Result::Right(r) => {
        let string_to_push = format!("\x1b[1;92m{r}");
        post.push_str(&string_to_push);
      },
    }
  }

  // push removed suffix
  prev.push_str(&format!("\x1b[33m{}", &suffix));
  post.push_str(&format!("\x1b[33m{}", &suffix));

  // reset
  prev.push_str("\x1b[0m");
  post.push_str("\x1b[0m");

  (prev, post)
}

/// print the packages added between two closures.
pub fn print_added(
  set: &HashSet<&str>,
  post: &HashMap<&str, HashSet<&str>>,
  col_width: usize,
) {
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
      "[{}] {:col_width$} \x1b[33m{}\x1b[0m",
      "A:".green().bold(),
      p,
      version_str
    );
  }
}

/// print the packages removed between two closures.
pub fn print_removed(
  set: &HashSet<&str>,
  pre: &HashMap<&str, HashSet<&str>>,
  col_width: usize,
) {
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
      "[{}] {:col_width$} \x1b[33m{}\x1b[0m",
      "R:".red().bold(),
      p,
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
  println!("{}", "Versions changed:".underline().bold());

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
  changes.sort_by(|(a, ..), (b, ..)| a.cmp(b));

  for (p, ver_pre, ver_post) in changes {
    let mut version_vec_pre =
      ver_pre.difference(ver_post).copied().collect::<Vec<_>>();
    let mut version_vec_post =
      ver_post.difference(ver_pre).copied().collect::<Vec<_>>();

    version_vec_pre.sort_unstable();
    version_vec_post.sort_unstable();

    let mut diffed_pre: String;
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
      (diffed_pre, diffed_post) =
        diff_versions(&version_str_pre, &version_str_post);
    }

    // push a space to the diffed_pre, if it is non-empty, we do this here and
    // not in the println in order to properly align the ±.
    if !version_vec_pre.is_empty() {
      let mut tmp = " ".to_string();
      tmp.push_str(&diffed_pre);
      diffed_pre = tmp;
    }

    println!(
      "[{}] {:col_width$}{} \x1b[0m\u{00B1}\x1b[0m {}",
      "C:".bold().bright_yellow(),
      p,
      diffed_pre,
      diffed_post
    );
  }
}

// Returns a reference to the compiled regex pattern.
// The regex is compiled only once.
fn name_regex() -> &'static Regex {
  static REGEX: OnceLock<Regex> = OnceLock::new();
  REGEX.get_or_init(|| {
    Regex::new(r"(-man|-lib|-doc|-dev|-out|-terminfo)")
      .expect("Failed to compile regex pattern for name")
  })
}
