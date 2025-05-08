use std::cmp;

use derive_more::{
  Deref,
  DerefMut,
  From,
};
use ref_cast::RefCast;

#[derive(RefCast, Deref, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct Version(str);

impl PartialOrd for Version {
  fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl cmp::Ord for Version {
  fn cmp(&self, that: &Self) -> cmp::Ordering {
    let this = VersionComponentIter::from(&**self);
    let that = VersionComponentIter::from(&**that);

    this.cmp(that)
  }
}

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

#[cfg(test)]
mod tests {
  use crate::version::{
    VersionComponent::{
      Number,
      Text,
    },
    VersionComponentIter,
  };

  #[test]
  fn version_component_iter() {
    let version = "132.1.2test234-1-man----.--.......---------..---";

    assert_eq!(VersionComponentIter::from(version).collect::<Vec<_>>(), [
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
