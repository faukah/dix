use std::cmp;

use derive_more::{
  Deref,
  DerefMut,
  Display,
  From,
};

#[derive(Deref, DerefMut, Display, Debug, Clone, PartialEq, Eq, From)]
pub struct Version(String);

impl PartialOrd for Version {
  fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl cmp::Ord for Version {
  fn cmp(&self, that: &Self) -> cmp::Ordering {
    let this = VersionComponentIter::from(&***self).filter_map(Result::ok);
    let that = VersionComponentIter::from(&***that).filter_map(Result::ok);

    this.cmp(that)
  }
}

impl<'a> IntoIterator for &'a Version {
  type Item = Result<VersionComponent<'a>, &'a str>;

  type IntoIter = VersionComponentIter<'a>;

  fn into_iter(self) -> Self::IntoIter {
    VersionComponentIter::from(&***self)
  }
}

#[derive(Display, Debug, Clone, Copy, Eq, PartialEq)]
pub enum VersionComponent<'a> {
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

    match (*self, *other) {
      (Number(this), Number(that)) => this.cmp(&that),
      (Text(this), Text(that)) => {
        match (this, that) {
          ("pre", _) => cmp::Ordering::Less,
          (_, "pre") => cmp::Ordering::Greater,
          _ => this.cmp(that),
        }
      },
      (Text(_), Number(_)) => cmp::Ordering::Less,
      (Number(_), Text(_)) => cmp::Ordering::Greater,
    }
  }
}

/// Yields [`VertionComponent`] from a version string.
#[derive(Deref, DerefMut, From)]
pub struct VersionComponentIter<'a>(&'a str);

impl<'a> Iterator for VersionComponentIter<'a> {
  type Item = Result<VersionComponent<'a>, &'a str>;

  fn next(&mut self) -> Option<Self::Item> {
    if self.starts_with(['.', '-', '*', ' ']) {
      let ret = &self[..1];
      **self = &self[1..];
      return Some(Err(ret));
    }

    // Get the next character and decide if it is a digit.
    let is_digit = self.chars().next()?.is_ascii_digit();

    // Based on this collect characters after this into the component.
    let component_len = self
      .chars()
      .take_while(|&char| {
        char.is_ascii_digit() == is_digit
          && !matches!(char, '.' | '-' | '*' | ' ')
      })
      .map(char::len_utf8)
      .sum();

    let component = &self[..component_len];
    **self = &self[component_len..];

    assert!(!component.is_empty());

    if is_digit {
      component
        .parse::<u64>()
        .ok()
        .map(VersionComponent::Number)
        .map(Ok)
    } else {
      Some(Ok(VersionComponent::Text(component)))
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

    assert_eq!(
      VersionComponentIter::from(version)
        .filter_map(Result::ok)
        .collect::<Vec<_>>(),
      [
        Number(132),
        Number(1),
        Number(2),
        Text("test"),
        Number(234),
        Number(1),
        Text("man")
      ]
    );
  }
}
