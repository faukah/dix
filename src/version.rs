use std::{
  cmp,
  fmt,
};

use derive_more::{
  Deref,
  Display,
};

/// Separators used to split version strings.
const SEPARATORS: &[char] = &['.', '-', '_', '+', '*', '=', '×', ' '];

/// A version string with semantic comparison support.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version {
  pub name:   String,
  pub amount: usize,
}

impl Version {
  pub fn new(version: impl Into<String>) -> Self {
    Self {
      name:   version.into(),
      amount: 1,
    }
  }

  /// Iterate over components only.
  pub fn components(&self) -> impl Iterator<Item = VersionComponent<'_>> {
    Pieces::new(&self.name).filter_map(VersionPiece::component)
  }

  /// Iterate over all pieces (components and separators).
  #[must_use]
  pub fn iter(&self) -> Pieces<'_> {
    Pieces::new(&self.name)
  }
}

impl<T: Into<String>> From<T> for Version {
  fn from(s: T) -> Self {
    Self::new(s)
  }
}

impl PartialOrd for Version {
  fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl Ord for Version {
  fn cmp(&self, other: &Self) -> cmp::Ordering {
    let self_comps: Vec<_> = self.components().collect();
    let other_comps: Vec<_> = other.components().collect();

    let min_len = self_comps.len().min(other_comps.len());

    // Compare common prefix
    for i in 0..min_len {
      let ord = self_comps[i].cmp(&other_comps[i]);
      if ord != cmp::Ordering::Equal {
        return ord;
      }
    }

    // Equal so far - check for pre-release semantics
    match self_comps.len().cmp(&other_comps.len()) {
      cmp::Ordering::Equal => cmp::Ordering::Equal,
      cmp::Ordering::Greater => {
        // Self has extra components - if they're non-numeric, self is a
        // pre-release
        if self_comps[min_len..].iter().any(|c| !c.is_numeric()) {
          cmp::Ordering::Less
        } else {
          cmp::Ordering::Greater
        }
      },
      cmp::Ordering::Less => {
        // Other has extra components - if they're non-numeric, other is a
        // pre-release
        if other_comps[min_len..].iter().any(|c| !c.is_numeric()) {
          cmp::Ordering::Greater
        } else {
          cmp::Ordering::Less
        }
      },
    }
  }
}

impl<'a> IntoIterator for &'a Version {
  type Item = VersionPiece<'a>;
  type IntoIter = Pieces<'a>;

  fn into_iter(self) -> Self::IntoIter {
    Pieces::new(&self.name)
  }
}

/// Iterator over version pieces (components and separators).
#[derive(Clone, Copy)]
pub struct Pieces<'a> {
  remaining: &'a str,
}

impl<'a> Pieces<'a> {
  const fn new(s: &'a str) -> Self {
    Self { remaining: s }
  }
}

#[allow(clippy::copy_iterator)]
impl<'a> Iterator for Pieces<'a> {
  type Item = VersionPiece<'a>;

  fn next(&mut self) -> Option<Self::Item> {
    if self.remaining.is_empty() {
      return None;
    }

    let first = self.remaining.chars().next()?;

    if SEPARATORS.contains(&first) {
      let len = first.len_utf8();
      let sep = &self.remaining[..len];
      self.remaining = &self.remaining[len..];
      return Some(VersionPiece::Separator(sep));
    }

    let len = self
      .remaining
      .chars()
      .take_while(|c| !SEPARATORS.contains(c))
      .map(char::len_utf8)
      .sum();

    let comp = &self.remaining[..len];
    self.remaining = &self.remaining[len..];
    Some(VersionPiece::Component(VersionComponent(comp)))
  }
}

/// Either a component or separator from a version string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionPiece<'a> {
  Component(VersionComponent<'a>),
  Separator(&'a str),
}

impl<'a> VersionPiece<'a> {
  #[must_use]
  pub const fn component(self) -> Option<VersionComponent<'a>> {
    match self {
      VersionPiece::Component(c) => Some(c),
      VersionPiece::Separator(_) => None,
    }
  }

  #[must_use]
  pub const fn separator(self) -> Option<&'a str> {
    match self {
      VersionPiece::Component(_) => None,
      VersionPiece::Separator(s) => Some(s),
    }
  }
}

/// A single version component (numeric or text).
#[derive(Display, Debug, Clone, Copy, Deref, PartialEq, Eq)]
pub struct VersionComponent<'a>(&'a str);

impl<'a> VersionComponent<'a> {
  pub fn is_numeric(&self) -> bool {
    !self.0.is_empty() && self.0.bytes().all(|b| b.is_ascii_digit())
  }

  pub fn as_u64(&self) -> Option<u64> {
    if self.is_numeric() {
      self.0.parse().ok()
    } else {
      None
    }
  }
}

impl PartialOrd for VersionComponent<'_> {
  fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl Ord for VersionComponent<'_> {
  fn cmp(&self, other: &Self) -> cmp::Ordering {
    match (self.is_numeric(), other.is_numeric()) {
      (true, true) => {
        match (self.as_u64(), other.as_u64()) {
          (Some(a), Some(b)) => a.cmp(&b),
          _ => self.0.cmp(other.0),
        }
      },
      (false, false) => {
        match (self.0, other.0) {
          ("pre", _) => cmp::Ordering::Less,
          (_, "pre") => cmp::Ordering::Greater,
          _ => self.0.cmp(other.0),
        }
      },
      (true, false) => cmp::Ordering::Less,
      (false, true) => cmp::Ordering::Greater,
    }
  }
}

impl fmt::Display for Version {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if self.amount > 1 {
      write!(f, "{} ×{}", self.name, self.amount)
    } else {
      f.write_str(&self.name)
    }
  }
}

impl fmt::Write for Version {
  fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> fmt::Result {
    fmt::write(&mut self.name, args)
  }

  fn write_str(&mut self, s: &str) -> fmt::Result {
    self.name.push_str(s);
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::{
    Version,
    VersionComponent,
    VersionPiece,
  };

  #[test]
  fn version_component_iter() {
    let version = "132.1.2test234-1-man----.--.......---------..---";

    assert_eq!(
      Version::new(version)
        .into_iter()
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
    assert!(Version::new("2.0.0") > Version::new("1.9.9"));
    assert!(Version::new("2.1.0") > Version::new("2.0.9"));
    assert!(Version::new("2.0.1") > Version::new("2.0.0"));
    assert!(Version::new("1.0.0") > Version::new("1.0.0-pre"));
    assert!(Version::new("1.0.0-beta") > Version::new("1.0.0-alpha"));
    assert!(Version::new("1.0.0-beta.11") > Version::new("1.0.0-beta.2"));
    assert_eq!(Version::new("1.0.0"), Version::new("1.0.0"));
  }
}
