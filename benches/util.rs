mod common;

use common::get_packages;
use criterion::{
  Criterion,
  black_box,
  criterion_group,
  criterion_main,
};
use dix::util::PackageDiff;

pub fn bench_package_diff(c: &mut Criterion) {
  let (pkgs_before, pkgs_after) = get_packages();
  c.bench_function("PackageDiff::new", |b| {
    b.iter(|| PackageDiff::new(black_box(pkgs_before), black_box(pkgs_after)));
  });
}

criterion_group!(benches, bench_package_diff);
criterion_main!(benches);
