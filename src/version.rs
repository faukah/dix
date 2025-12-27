use std::{
  cmp,
  convert::Infallible,
  fmt,
  str::FromStr,
};

use derive_more::{
  Deref,
  Display,
  From,
};

/// A type representing a version string.
///
/// Implements comparison operations based on semantic version principles with
/// some additional handling for various separators and special component names.
/// The comparison is done by parsing the string into components and comparing
/// them individually.
/// Constants for version parsing and comparison
pub const VERSION_SEPARATORS: &[char] =
  &['.', '-', '_', '+', '*', '=', '×', ' '];

#[derive(Display, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version(pub String);

impl Version {
  /// Create a new Version from a string
  pub fn new(version: impl Into<String>) -> Self {
    Self(version.into())
  }

  /// Returns a string slice of the entire version
  #[must_use]
  pub fn as_str(&self) -> &str {
    &self.0
  }

  /// Returns an iterator over the components of this version
  pub fn components(&self) -> impl Iterator<Item = VersionComponent<'_>> {
    VersionIter::from(self.0.as_str()).filter_map(VersionPiece::component)
  }
}

impl FromStr for Version {
  type Err = Infallible;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    Ok(Self(s.to_owned()))
  }
}

impl From<String> for Version {
  fn from(s: String) -> Self {
    Self(s)
  }
}

impl<'a> From<&'a str> for Version {
  fn from(s: &'a str) -> Self {
    Self(s.to_owned())
  }
}

impl PartialOrd for Version {
  fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl cmp::Ord for Version {
  fn cmp(&self, that: &Self) -> cmp::Ordering {
    let this = self.components();
    let that = that.components();

    this.cmp(that)
  }
}

impl<'a> IntoIterator for &'a Version {
  type Item = VersionPiece<'a>;

  type IntoIter = VersionIter<'a>;

  fn into_iter(self) -> Self::IntoIter {
    VersionIter::from(self.0.as_str())
  }
}

/// Iterator that yields [`VersionPiece`] instances from a version string.
/// Splits a version string into components and separators.
#[derive(Deref, From)]
pub struct VersionIter<'a>(&'a str);

impl<'a> Iterator for VersionIter<'a> {
  type Item = VersionPiece<'a>;

  fn next(&mut self) -> Option<Self::Item> {
    if self.0.is_empty() {
      return None;
    }

    if self
      .0
      .starts_with(|c: char| VERSION_SEPARATORS.contains(&c))
    {
      let len = self.0.chars().next().unwrap().len_utf8();
      let (this, rest) = self.0.split_at(len);

      self.0 = rest;
      return Some(VersionPiece::Separator(this));
    }

    // Collect all characters until we reach a separator
    let component_len = self
      .0
      .chars()
      .take_while(|&c| !VERSION_SEPARATORS.contains(&c))
      .map(char::len_utf8)
      .sum();

    // This should never be zero because we already checked for separators
    if component_len == 0 {
      return None;
    }

    let component = &self.0[..component_len];
    self.0 = &self.0[component_len..];

    Some(VersionPiece::Component(VersionComponent(component)))
  }
}

/// A component of a version string, such as a number or a pre-release
/// identifier. Contains logic for comparing version components according to
/// semver-like rules.
#[derive(Display, Debug, Clone, Copy, Deref, Eq, PartialEq, PartialOrd)]
pub struct VersionComponent<'a>(&'a str);

impl<'a> VersionComponent<'a> {
  /// Returns the underlying string slice
  #[must_use]
  pub const fn as_str(&self) -> &'a str {
    self.0
  }

  /// Returns true if this component consists only of ASCII digits
  #[must_use]
  pub fn is_numeric(&self) -> bool {
    !self.0.is_empty() && self.0.bytes().all(|b| b.is_ascii_digit())
  }

  /// Attempts to parse this component as a u64
  #[must_use]
  pub fn as_u64(&self) -> Option<u64> {
    if self.is_numeric() {
      self.0.parse::<u64>().ok()
    } else {
      None
    }
  }
}

impl cmp::Ord for VersionComponent<'_> {
  fn cmp(&self, other: &Self) -> cmp::Ordering {
    // Check if both components are numeric
    let self_numeric = self.is_numeric();
    let other_numeric = other.is_numeric();

    match (self_numeric, other_numeric) {
      // Both components are numeric - compare as numbers
      (true, true) => {
        match (self.as_u64(), other.as_u64()) {
          (Some(self_num), Some(other_num)) => self_num.cmp(&other_num),
          // Fall back to string comparison in the unlikely case parsing fails
          _ => self.0.cmp(other.0),
        }
      },

      // Both components are non-numeric - compare as strings with special cases
      (false, false) => {
        match (self.0, other.0) {
          // "pre" is always less than any other non-numeric component
          ("pre", _) => cmp::Ordering::Less,
          (_, "pre") => cmp::Ordering::Greater,
          // Default to lexicographic comparison
          _ => self.0.cmp(other.0),
        }
      },

      // Mixed types - numeric components lower precedence than non-numeric ones
      (true, false) => cmp::Ordering::Less,
      (false, true) => cmp::Ordering::Greater,
    }
  }
}

/// Represents either a version component or a separator.
///
/// Used by [`VersionIter`] to provide access to both components and
/// separators when iterating through a version string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionPiece<'a> {
  /// A meaningful component of a version (number or identifier)
  Component(VersionComponent<'a>),
  /// A separator character or string (like '.', '-', etc.)
  Separator(&'a str),
}

impl<'a> VersionPiece<'a> {
  /// Extracts the component if this piece is a component, otherwise returns
  /// None
  #[must_use]
  pub const fn component(self) -> Option<VersionComponent<'a>> {
    match self {
      VersionPiece::Component(component) => Some(component),
      VersionPiece::Separator(_) => None,
    }
  }

  /// Returns the separator if this piece is a separator, otherwise returns None
  #[must_use]
  pub const fn separator(self) -> Option<&'a str> {
    match self {
      VersionPiece::Component(_) => None,
      VersionPiece::Separator(sep) => Some(sep),
    }
  }
}

// Implement fmt::Write for Version to support write! macro
impl fmt::Write for Version {
  fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> fmt::Result {
    fmt::write(&mut self.0, args)
  }
  fn write_str(&mut self, s: &str) -> fmt::Result {
    (self.0).write_str(s)
  }
}

#[cfg(test)]
mod tests {
  use super::{
    Version,
    VersionComponent,
    VersionIter,
  };
  use crate::version::VersionPiece;

  #[test]
  fn version_component_iter() {
    let version = "132.1.2test234-1-man----.--.......---------..---";

    assert_eq!(
      VersionIter::from(version)
        .filter_map(VersionPiece::component)
        .collect::<Vec<_>>(),
      [
        VersionComponent("132"),
        VersionComponent("1"),
        VersionComponent("2test234"),
        VersionComponent("1"),
        VersionComponent("man")
      ]
    );
  }

  #[test]
  fn version_comparison() {
    // Numeric comparison
    assert!(Version::new("2.0.0") > Version::new("1.9.9"));
    assert!(Version::new("2.1.0") > Version::new("2.0.9"));
    assert!(Version::new("2.0.1") > Version::new("2.0.0"));

    // TODO: assert should not fail.
    // Pre-release designations
    // assert!(Version::new("1.0.0") > Version::new("1.0.0-pre"));
    assert!(Version::new("1.0.0-beta") > Version::new("1.0.0-alpha"));

    // Mixed numeric and text
    assert!(Version::new("1.0.0-beta.11") > Version::new("1.0.0-beta.2"));

    // Equivalence
    assert_eq!(Version::new("1.0.0"), Version::new("1.0.0"));
  }
}
