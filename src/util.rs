use std::cmp::Ordering;

#[derive(Eq, PartialEq, Debug)]
enum VersionComponent {
    Number(u64),
    Text(String),
}

impl std::cmp::Ord for VersionComponent {
    fn cmp(&self, other: &Self) -> Ordering {
        use VersionComponent::{Number, Text};
        match (self, other) {
            (Number(x), Number(y)) => x.cmp(y),
            (Text(x), Text(y)) => match (x.as_str(), y.as_str()) {
                ("pre", _) => Ordering::Less,
                (_, "pre") => Ordering::Greater,
                _ => x.cmp(y),
            },
            (Text(_), Number(_)) => Ordering::Less,
            (Number(_), Text(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for VersionComponent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// takes a version string and outputs the different components
//
// a component is delimited by '-' or '.' and consists of just digits or letters
struct VersionComponentIterator<'a> {
    v: &'a [u8],
    pos: usize,
}

impl<'a> VersionComponentIterator<'a> {
    pub fn new<I: Into<&'a str>>(v: I) -> Self {
        Self {
            v: v.into().as_bytes(),
            pos: 0,
        }
    }
}

impl Iterator for VersionComponentIterator<'_> {
    type Item = VersionComponent;

    fn next(&mut self) -> Option<Self::Item> {
        // skip all '-' and '.' in the beginning
        while let Some(b'.' | b'-') = self.v.get(self.pos) {
            self.pos += 1;
        }

        // get the next character and decide if it is a digit or char
        let c = self.v.get(self.pos)?;
        let is_digit = c.is_ascii_digit();
        // based on this collect characters after this into the component
        let component_len = self.v[self.pos..]
            .iter()
            .copied()
            .take_while(|&c| c.is_ascii_digit() == is_digit && c != b'.' && c != b'-')
            .count();
        let component =
            String::from_utf8_lossy(&self.v[self.pos..(self.pos + component_len)]).into_owned();

        // remember what chars we used
        self.pos += component_len;

        if component.is_empty() {
            None
        } else if is_digit {
            component.parse::<u64>().ok().map(VersionComponent::Number)
        } else {
            Some(VersionComponent::Text(component))
        }
    }
}

/// Compares two strings of package versions, and figures out the greater one.
///
/// # Returns
///
/// * Ordering
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let iter_a = VersionComponentIterator::new(a);
    let iter_b = VersionComponentIterator::new(b);

    iter_a.cmp(iter_b)
}

mod test {

    #[test]
    fn test_version_component_iter() {
        use super::VersionComponent::{Number, Text};
        use crate::util::VersionComponentIterator;
        let v = "132.1.2test234-1-man----.--.......---------..---";

        let comp: Vec<_> = VersionComponentIterator::new(v).collect();
        assert_eq!(
            comp,
            [
                Number(132),
                Number(1),
                Number(2),
                Text("test".into()),
                Number(234),
                Number(1),
                Text("man".into())
            ]
        );
    }
}
