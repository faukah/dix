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
enum VersionComponent<'a> {
  Number(u64),
  Text(&'a str),
}

impl PartialOrd for VersionComponent<'_> {
  fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl cmp::Ord for VersionComponent<'_> {
  fn cmp(&self, other: &Self) -> cmp::Ordering {
    use VersionComponent::{
      Number,
      Text,
    };

    match (self, other) {
      (Number(x), Number(y)) => x.cmp(y),
      (Text(x), Text(y)) => {
        match (*x, *y) {
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

impl<'a> Iterator for VersionComponentIter<'a> {
  type Item = VersionComponent<'a>;

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

    let component = &self[..component_len];
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

/// Parses a Nix store path to extract the packages name and possibly its
/// version.
///
/// This function first drops the inputs first 44 chars, since that is exactly
/// the length of the `/nix/store/0004yybkm5hnwjyxv129js3mjp7kbrax-` prefix.
/// Then it matches that against our store path regex.
pub fn parse_name_and_version(
  path: &StorePath,
) -> Result<(&str, Option<&str>)> {
  static STORE_PATH_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(.+?)(-([0-9].*?))?$")
      .expect("failed to compile regex pattern for nix store paths")
  });

  let path = path.to_str().with_context(|| {
    format!(
      "failed to convert path '{path}' to valid unicode",
      path = path.display(),
    )
  })?;

  // We can strip the path since it _always_ follows the format:
  //
  // /nix/store/0004yybkm5hnwjyxv129js3mjp7kbrax-...
  // ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
  // This part is exactly 44 chars long, so we just remove it.
  assert_eq!(&path[..11], "/nix/store/");
  assert_eq!(&path[43..44], "-");
  let path = &path[44..];

  log::debug!("stripped path: {path}");

  let captures = STORE_PATH_REGEX.captures(path).ok_or_else(|| {
    anyhow!("path '{path}' does not match expected Nix store format")
  })?;

  let name = captures.get(1).map_or("", |m| m.as_str());
  if name.is_empty() {
    bail!("failed to extract name from path '{path}'");
  }

  let version = captures.get(2).map(|m| m.as_str().trim_start_matches('-'));

  Ok((name, version))
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
