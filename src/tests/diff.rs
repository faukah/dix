use std::{
  fmt,
  io,
};

use size::Size;

use crate::{
  Version,
  diff,
};

struct WriteFmt<W: io::Write>(W);

impl<W: io::Write> fmt::Write for WriteFmt<W> {
  fn write_str(&mut self, string: &str) -> fmt::Result {
    self.0.write_all(string.as_bytes()).map_err(|_| fmt::Error)
  }
}

#[test]
fn test_deduplicate_versions() {
  let mut versions_pre: Vec<Version> = vec![
    "2.3".into(),
    "1.0".into(),
    "2.3".into(),
    "4.8".into(),
    "2.3".into(),
    "1.0".into(),
  ];
  let versions_post: Vec<Version> =
    vec!["1.0 ×2".into(), "2.3 ×3".into(), "4.8".into()];
  diff::deduplicate_versions(&mut versions_pre);

  assert_eq!(versions_pre, versions_post);
}

#[test]
fn test_deduplicate_versions_empty() {
  let mut versions_pre: Vec<Version> = vec![];
  let versions_post: Vec<Version> = vec![];
  diff::deduplicate_versions(&mut versions_pre);

  assert_eq!(versions_pre, versions_post);
}

#[test]
fn test_write_size_diffln() {
  let size_old = Size::from_bytes(10_i32);
  let size_new = Size::from_bytes(20_i32);

  let expected_output =
    "\u{1b}[1mSIZE\u{1b}[0m: \u{1b}[31m10 bytes\u{1b}[0m -> \u{1b}[32m20 \
     bytes\u{1b}[0m\n\u{1b}[1mDIFF\u{1b}[0m: \u{1b}[32m10 bytes\u{1b}[0m\n";

  let mut buf = WriteFmt(io::BufWriter::new(Vec::new()));

  let _ = diff::write_size_diffln(&mut buf, size_old, size_new);

  let buf_writer = buf.0;

  let vec = buf_writer.into_inner().expect("Failed to unwrap BufWriter");
  let result = String::from_utf8(vec).expect("Invalid UTF-8");

  assert_eq!(result, expected_output);
}

#[test]
fn test_write_size_diffln_empty() {
  let size_old = Size::from_bytes(0_i32);
  let size_new = Size::from_bytes(0_i32);

  let expected_output =
    "\u{1b}[1mSIZE\u{1b}[0m: \u{1b}[31m0 bytes\u{1b}[0m -> \u{1b}[32m0 \
     bytes\u{1b}[0m\n\u{1b}[1mDIFF\u{1b}[0m: \u{1b}[31m0 bytes\u{1b}[0m\n";
  let mut buf = WriteFmt(io::BufWriter::new(Vec::new()));

  let _ = diff::write_size_diffln(&mut buf, size_old, size_new);

  let buf_writer = buf.0;

  let vec = buf_writer.into_inner().expect("Failed to unwrap BufWriter");
  let result = String::from_utf8(vec).expect("Invalid UTF-8");

  assert_eq!(result, expected_output);
}
