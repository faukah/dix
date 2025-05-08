use std::{
  cmp::Ordering,
  collections::{
    HashMap,
    HashSet,
  },
  sync::OnceLock,
};

use log::debug;
use regex::Regex;

use crate::error::AppError;

// Use type alias for Result with our custom error type
type Result<T> = std::result::Result<T, AppError>;

use std::string::ToString;

#[derive(Eq, PartialEq, Debug)]
#[derive(Debug, Clone, Eq, PartialEq)]
enum VersionComponent {
  Number(u64),
  Text(String),
}

impl std::cmp::Ord for VersionComponent {
  fn cmp(&self, other: &Self) -> Ordering {
    use VersionComponent::{
      Number,
      Text,
    };

    match (self, other) {
      (Number(x), Number(y)) => x.cmp(y),
      (Text(x), Text(y)) => {
        match (x.as_str(), y.as_str()) {
          ("pre", _) => cmp::Ordering::Less,
          (_, "pre") => cmp::Ordering::Greater,
          _ => x.cmp(y),
        }
      },
      (Text(_), Number(_)) => cmp::Ordering::Less,
      (Number(_), Text(_)) => cmp::Ordering::Greater,
    }
  }
}

/// Yields [`VertionComponent`] from a version string.
#[derive(Deref, DerefMut, From)]
struct VersionComponentIter<'a>(&'a str);

impl Iterator for VersionComponentIter<'_> {
  type Item = VersionComponent;

  fn next(&mut self) -> Option<Self::Item> {
    // Skip all '-' and '.'.
    while self.starts_with(['.', '-']) {
      **self = &self[1..];
    }

    // Get the next character and decide if it is a digit.
    let is_digit = self.chars().next()?.is_ascii_digit();

    // Based on this collect characters after this into the component.
    let component_len = self
      .chars()
      .take_while(|&c| {
        c.is_ascii_digit() == is_digit && !matches!(c, '.' | '-')
      })
      .map(char::len_utf8)
      .sum();

    let component = self[..component_len].to_owned();
    **self = &self[component_len..];

    assert!(!component.is_empty());

    if is_digit {
      component.parse::<u64>().ok().map(VersionComponent::Number)
    } else {
      Some(VersionComponent::Text(component))
    }
  }
}

/// Compares two strings of package versions, and figures out the greater one.
pub fn compare_versions(this: &str, that: &str) -> cmp::Ordering {
  let this = VersionComponentIter::from(this);
  let that = VersionComponentIter::from(that);

  this.cmp(that)
}

/// Parses a nix store path to extract the packages name and version
///
/// This function first drops the inputs first 44 chars, since that is exactly
/// the length of the /nix/store/... prefix. Then it matches that against our
/// store path regex.
///
/// # Returns
///
/// * Result<(&'a str, &'a str)> - The Package's name and version, or an error
///   if one or both cannot be retrieved.
pub fn get_version<'a>(pack: impl Into<&'a str>) -> Result<(&'a str, &'a str)> {
  let path = pack.into();

  // We can strip the path since it _always_ follows the format
  // /nix/store/<...>-<program_name>-......
  // This part is exactly 44 chars long, so we just remove it.
  let stripped_path = &path[44..];
  debug!("Stripped path: {stripped_path}");

  // Match the regex against the input
  if let Some(cap) = store_path_regex().captures(stripped_path) {
    // Handle potential missing captures safely
    let name = cap.get(1).map_or("", |m| m.as_str());
    let mut version = cap.get(2).map_or("<none>", |m| m.as_str());

    if version.starts_with('-') {
      version = &version[1..];
    }

    if name.is_empty() {
      return Err(AppError::ParseError {
        message: format!("Failed to extract name from path: {path}"),
        context: "get_version".to_string(),
        source:  None,
      });
    }

    return Ok((name, version));
  }

  Err(AppError::ParseError {
    message: format!("Path does not match expected nix store format: {path}"),
    context: "get_version".to_string(),
    source:  None,
  })
}

// Returns a reference to the compiled regex pattern.
// The regex is compiled only once.
pub fn store_path_regex() -> &'static Regex {
  static REGEX: OnceLock<Regex> = OnceLock::new();
  REGEX.get_or_init(|| {
    Regex::new(r"(.+?)(-([0-9].*?))?$")
      .expect("Failed to compile regex pattern for nix store paths")
  })
}

// TODO: move this somewhere else, this does not really
// belong into this file
pub struct PackageDiff<'a> {
  pub pkg_to_versions_pre:  HashMap<&'a str, HashSet<&'a str>>,
  pub pkg_to_versions_post: HashMap<&'a str, HashSet<&'a str>>,
  pub pre_keys:             HashSet<&'a str>,
  pub post_keys:            HashSet<&'a str>,
  pub added:                HashSet<&'a str>,
  pub removed:              HashSet<&'a str>,
  pub changed:              HashSet<&'a str>,
}

impl<'a> PackageDiff<'a> {
  pub fn new<S: AsRef<str> + 'a>(
    pkgs_pre: &'a [S],
    pkgs_post: &'a [S],
  ) -> Self {
    // Map from packages of the first closure to their version
    let mut pre = HashMap::<&str, HashSet<&str>>::new();
    let mut post = HashMap::<&str, HashSet<&str>>::new();

    for p in pkgs_pre {
      match get_version(p.as_ref()) {
        Ok((name, version)) => {
          pre.entry(name).or_default().insert(version);
        },
        Err(e) => {
          debug!("Error parsing package version: {e}");
        },
      }
    }

    for p in pkgs_post {
      match get_version(p.as_ref()) {
        Ok((name, version)) => {
          post.entry(name).or_default().insert(version);
        },
        Err(e) => {
          debug!("Error parsing package version: {e}");
        },
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
    Self {
      pkg_to_versions_pre: pre,
      pkg_to_versions_post: post,
      pre_keys,
      post_keys,
      added,
      removed,
      changed,
    }
  }
}

mod test {

  #[test]
  fn test_version_component_iter() {
    use super::VersionComponent::{
      Number,
      Text,
    };
    use crate::util::VersionComponentIter;
    let v = "132.1.2test234-1-man----.--.......---------..---";

    let comp: Vec<_> = VersionComponentIter::new(v).collect();
    assert_eq!(comp, [
      Number(132),
      Number(1),
      Number(2),
      Text("test".into()),
      Number(234),
      Number(1),
      Text("man".into())
    ]);
  }
}
