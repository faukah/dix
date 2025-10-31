use std::cmp;

use derive_more::{
  Deref,
  DerefMut,
  Display,
  From,
};

#[derive(Deref, DerefMut, Display, Debug, Clone, PartialEq, Eq, From)]
pub struct Version(pub String);

impl PartialOrd for Version {
  fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl cmp::Ord for Version {
  fn cmp(&self, that: &Self) -> cmp::Ordering {
    let this = VersionIter::from(&***self).filter_map(VersionPiece::component);
    let that = VersionIter::from(&***that).filter_map(VersionPiece::component);

    this.cmp(that)
  }
}

impl<'a> IntoIterator for &'a Version {
  type Item = VersionPiece<'a>;

  type IntoIter = VersionIter<'a>;

  fn into_iter(self) -> Self::IntoIter {
    VersionIter::from(&***self)
  }
}

/// Yields [`VersionPiece`] from a version string.
#[derive(Deref, DerefMut, From)]
pub struct VersionIter<'a>(&'a str);

impl<'a> Iterator for VersionIter<'a> {
  type Item = VersionPiece<'a>;

  fn next(&mut self) -> Option<Self::Item> {
    const SPLIT_CHARS: &[char] = &['.', '-', '_', '+', '*', '=', '×', ' '];

    if self.is_empty() {
      return None;
    }

    if self.starts_with(SPLIT_CHARS) {
      let len = self
        .chars()
        .next()
        .expect("self starts with a char, so there is one")
        .len_utf8();
      let (this, rest) = self.split_at(len);

      **self = rest;
      return Some(VersionPiece::Separator(this));
    }

    // Based on this collect characters after this into the component.
    let component_len = self
      .chars()
      .take_while(|&char| !SPLIT_CHARS.contains(&char))
      .map(char::len_utf8)
      .sum();

    let component = &self[..component_len];
    **self = &self[component_len..];

    assert!(!component.is_empty());

    Some(VersionPiece::Component(VersionComponent(component)))
  }
}

#[derive(Deref, Display, Debug, Clone, Copy)]
pub struct VersionComponent<'a>(&'a str);

impl PartialEq for VersionComponent<'_> {
  fn eq(&self, other: &Self) -> bool {
    self.cmp(other) == cmp::Ordering::Equal
  }
}

impl Eq for VersionComponent<'_> {}

impl PartialOrd for VersionComponent<'_> {
  fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl cmp::Ord for VersionComponent<'_> {
  fn cmp(&self, other: &Self) -> cmp::Ordering {
    let self_digit = self.0.bytes().all(|char| char.is_ascii_digit());
    let other_digit = other.0.bytes().all(|char| char.is_ascii_digit());

    match (self_digit, other_digit) {
      (true, true) => {
        let self_nonzero = self.0.trim_start_matches('0');
        let other_nonzero = other.0.trim_start_matches('0');

        self_nonzero
          .len()
          .cmp(&other_nonzero.len())
          .then_with(|| self_nonzero.cmp(other_nonzero))
      },

      (false, false) => {
        match (self.0, other.0) {
          ("pre", _) => cmp::Ordering::Less,
          (_, "pre") => cmp::Ordering::Greater,
          _ => self.0.cmp(other.0),
        }
      },

      (true, false) => cmp::Ordering::Greater,
      (false, true) => cmp::Ordering::Less,
    }
  }
}

/// Used by the [`VersionComponentIter`] to still give access to separators
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionPiece<'a> {
  Component(VersionComponent<'a>),
  Separator(&'a str),
}

impl<'a> VersionPiece<'a> {
  pub fn component(self) -> Option<VersionComponent<'a>> {
    let VersionPiece::Component(component) = self else {
      return None;
    };

    Some(component)
  }
}

#[cfg(test)]
mod tests {
  use proptest::proptest;

  use super::{
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

  proptest! {
    #[test]
    fn version_cmp_number(this: u128, that: u128) {
      let real_ord = this.cmp(&that);

      let component_ord = VersionComponent(&this.to_string())
        .cmp(&VersionComponent(&that.to_string()));

      assert_eq!(real_ord, component_ord);
    }
  }
}
