use regex::Regex;
use std::cmp::Ordering;
use std::sync::OnceLock;

#[derive(Eq, PartialEq)]
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

/// Compares two strings of package versions, and figures out the greater one.
///
/// # Returns
///
/// * Ordering
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let iter_a = version_split_regex().find_iter(a).map(|m| {
        use VersionComponent::{Number, Text};
        let bla = m.as_str();
        bla.parse().map_or_else(|_| Text(bla.to_string()), Number)
    });

    let iter_b = version_split_regex().find_iter(b).map(|m| {
        use VersionComponent::{Number, Text};
        let bla = m.as_str();
        bla.parse().map_or_else(|_| Text(bla.to_string()), Number)
    });

    iter_a.cmp(iter_b)
}

fn version_split_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(\d+|[a-zA-Z]+)").expect("Failed to compile regex pattern for nix store paths")
    })
}
