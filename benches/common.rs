use std::sync::OnceLock;

use dixlib::{store, util::PackageDiff};

pub const QUERY_DERIV: &str = "/run/current-system/system";
//FIXME: Use some copy of the db so the queries work on another system!
pub const QUERY_DERIV_OLD: &str = "/nix/var/nix/profiles/system-825-link";

pub fn get_packages() -> &'static (Vec<String>, Vec<String>) {
    static _PKGS: OnceLock<(Vec<String>, Vec<String>)> = OnceLock::new();
    _PKGS.get_or_init(|| {
        let pkgs_before = store::get_packages(std::path::Path::new(QUERY_DERIV_OLD))
            .unwrap()
            .into_iter()
            .map(|(_, name)| name)
            .collect::<Vec<String>>();
        let pkgs_after = store::get_packages(std::path::Path::new(QUERY_DERIV))
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
