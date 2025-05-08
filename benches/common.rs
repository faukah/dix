use std::{
  env,
  fs,
  path::PathBuf,
  sync::OnceLock,
};

use dix::{
  store,
  util::PackageDiff,
};

/// tries to get the path of the oldest nixos system derivation
/// this function is pretty hacky and only used so that
/// you don't have to specify a specific derivation to
/// run the benchmarks
fn get_oldest_nixos_system() -> Option<PathBuf> {
  let profile_dir = fs::read_dir("/nix/var/nix/profiles").ok()?;

  let files = profile_dir.filter_map(Result::ok).filter_map(|entry| {
    entry
      .file_type()
      .ok()
      .and_then(|f| f.is_symlink().then_some(entry.path()))
  });

  files.min_by_key(|path| {
    // extract all digits from the file name and use that as key
    let p = path.as_os_str().to_str().unwrap_or_default();
    let digits: String = p.chars().filter(|c| c.is_ascii_digit()).collect();
    // if we are not able to produce a key (e.g. because the path does not
    // contain digits) we put it last
    digits.parse::<u32>().unwrap_or(u32::MAX)
  })
}

pub fn get_deriv_query() -> &'static PathBuf {
  static _QUERY_DERIV: OnceLock<PathBuf> = OnceLock::new();
  _QUERY_DERIV.get_or_init(|| {
    let path = PathBuf::from(
      env::var("DIX_BENCH_NEW_SYSTEM")
        .unwrap_or_else(|_| "/run/current-system/system".into()),
    );
    path
  })
}
pub fn get_deriv_query_old() -> &'static PathBuf {
  static _QUERY_DERIV: OnceLock<PathBuf> = OnceLock::new();
  _QUERY_DERIV.get_or_init(|| {
    let path = env::var("DIX_BENCH_OLD_SYSTEM")
      .ok()
      .map(PathBuf::from)
      .or(get_oldest_nixos_system())
      .unwrap_or_else(|| PathBuf::from("/run/current-system/system"));
    path
  })
}

pub fn get_packages() -> &'static (Vec<String>, Vec<String>) {
  static _PKGS: OnceLock<(Vec<String>, Vec<String>)> = OnceLock::new();
  _PKGS.get_or_init(|| {query_depdendents
    let pkgs_before =
      store::query_packages(std::path::Path::new(get_deriv_query_old()))
        .unwrap()
        .into_iter()
        .map(|(_, name)| name)query_depdendents
        .collect::<Vec<String>>();
    let pkgs_after =
      store::query_packages(std::path::Path::new(get_deriv_query()))
        .unwrap()
        .into_iter()
        .map(|(_, name)| name)
        .collect::<Vec<String>>();
    (pkgs_before, pkgs_after)
  })
}

pub fn get_pkg_diff() -> &'static PackageDiff<'static> {
  static _PKG_DIFF: OnceLock<PackageDiff> = OnceLock::new();
  _PKG_DIFF.get_or_init(|| {
    let (pkgs_before, pkgs_after) = get_packages();
    PackageDiff::new(pkgs_before, pkgs_after)
  })
}

/// prints the old and new NixOs system used for benchmarking
///
/// is used to give information about the old and new system
pub fn print_used_nixos_systems() {
  let old = get_deriv_query_old();
  let new = get_deriv_query();
  println!("old system used {:?}", old);
  println!("new system used {:?}", new);
}
