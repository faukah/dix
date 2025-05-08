mod common;

use std::{
  fs::File,
  os::fd::AsRawFd,
};

use common::{
  get_pkg_diff,
  print_used_nixos_systems,
};
use criterion::{
  Criterion,
  black_box,
  criterion_group,
  criterion_main,
};
use dix::print;

/// reroutes stdout and stderr to the null device before
/// executing `f`
fn suppress_output<F: FnOnce()>(f: F) {
  let stdout = std::io::stdout();
  let stderr = std::io::stderr();

  // Save original FDs
  let orig_stdout_fd = stdout.as_raw_fd();
  let orig_stderr_fd = stderr.as_raw_fd();

  // Open /dev/null and get its FD
  let devnull = File::create("/dev/null").unwrap();
  let null_fd = devnull.as_raw_fd();

  // Redirect stdout and stderr to /dev/null
  let _ = unsafe { libc::dup2(null_fd, orig_stdout_fd) };
  let _ = unsafe { libc::dup2(null_fd, orig_stderr_fd) };

  f();

  let _ = unsafe { libc::dup2(orig_stdout_fd, 1) };
  let _ = unsafe { libc::dup2(orig_stderr_fd, 2) };
}

pub fn bench_print_added(c: &mut Criterion) {
  print_used_nixos_systems();
  let diff = get_pkg_diff();
  c.bench_function("print_added", |b| {
    b.iter(|| {
      suppress_output(|| {
        print::print_added(
          black_box(&diff.added),
          black_box(&diff.pkg_to_versions_post),
          30,
        );
      });
    });
  });
}
pub fn bench_print_removed(c: &mut Criterion) {
  print_used_nixos_systems();
  let diff = get_pkg_diff();
  c.bench_function("print_removed", |b| {
    b.iter(|| {
      suppress_output(|| {
        print::print_removed(
          black_box(&diff.removed),
          black_box(&diff.pkg_to_versions_pre),
          30,
        );
      });
    });
  });
}
pub fn bench_print_changed(c: &mut Criterion) {
  print_used_nixos_systems();
  let diff = get_pkg_diff();
  c.bench_function("print_changed", |b| {
    b.iter(|| {
      suppress_output(|| {
        print::print_changes(
          black_box(&diff.changed),
          black_box(&diff.pkg_to_versions_pre),
          black_box(&diff.pkg_to_versions_post),
          30,
        );
      });
    });
  });
}

criterion_group!(
  benches,
  bench_print_added,
  bench_print_removed,
  bench_print_changed
);
criterion_main!(benches);
