use std::{
  path::{
    Path,
    PathBuf,
  },
  sync,
};

use derive_more::Deref;
use eyre::{
  Context as _,
  ContextCompat as _,
  Error,
  Result,
  bail,
  eyre,
};

#[cfg(feature = "json")] pub mod json;

pub mod diff;
pub use diff::{
  generate_diffs_from_paths,
  match_version_lists,
  spawn_size_diff,
  write_package_diff,
  write_packages_diff,
  write_size_diff,
};

mod store;

pub mod version;
use version::Version;

/// A validated store path. Always starts with `/nix/store`.
///
/// Can be created using `StorePath::try_from(path_buf)`.
#[derive(Deref, Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorePath(PathBuf);

impl TryFrom<PathBuf> for StorePath {
  type Error = Error;

  fn try_from(path: PathBuf) -> Result<Self> {
    tracing::trace!(path = %path.display(), "validating store path");
    if !path.starts_with("/nix/store") {
      tracing::warn!(path = %path.display(), "path does not start with /nix/store");
      bail!(
        "path {path} must start with /nix/store",
        path = path.display(),
      );
    }
    tracing::trace!(path = %path.display(), "store path validated");
    Ok(Self(path))
  }
}

impl StorePath {
  /// Parses a Nix store path to extract the packages name and possibly its
  /// version.
  ///
  /// This function first drops the inputs first 44 chars, since that is exactly
  /// the length of the `/nix/store/0004yybkm5hnwjyxv129js3mjp7kbrax-` prefix.
  /// Then it matches that against our store path regex.
  fn parse_name_and_version(&self) -> Result<(&str, Option<Version>)> {
    static STORE_PATH_REGEX: sync::LazyLock<regex::Regex> =
      sync::LazyLock::new(|| {
        regex::Regex::new("(.+?)(-([0-9].*?))?$")
          .expect("failed to compile regex for Nix store paths")
      });

    let path = self.to_str().with_context(|| {
      format!(
        "failed to convert path '{path}' to valid unicode",
        path = self.display(),
      )
    })?;

    // We can strip the path since it _always_ follows the format:
    //
    // /nix/store/0004yybkm5hnwjyxv129js3mjp7kbrax-...
    // ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    // This part is exactly 44 chars long, so we just remove it.
    assert_eq!(&path[..11], "/nix/store/");
    assert_eq!(&path[43..44], "-");
    let path = &path[44..];

    tracing::trace!("stripped path: {path}");

    let captures = STORE_PATH_REGEX.captures(path).ok_or_else(|| {
      eyre!("path '{path}' does not match expected Nix store format")
    })?;

    let name = captures.get(1).map_or("", |capture| capture.as_str());
    if name.is_empty() {
      bail!("failed to extract name from path '{path}'");
    }

    let version: Option<Version> = captures.get(2).map(|capture| {
      Version::from(capture.as_str().trim_start_matches('-').to_owned())
    });

    tracing::trace!(name = name, version = ?version, "parsed name and version from path");

    Ok((name, version))
  }
}

fn path_to_canonical_string(path: &Path) -> Result<String> {
  let path = path.canonicalize().with_context(|| {
    format!(
      "failed to canonicalize path '{path}'",
      path = path.display(),
    )
  })?;

  let path = path.into_os_string().into_string().map_err(|path| {
    tracing::debug!("path contains invalid unicode characters");
    eyre!(
      "failed to convert path '{path}' to valid unicode",
      path = Path::new(&*path).display(), /* TODO: use .display() directly
                                           * after Rust 1.87.0 in flake. */
    )
  })?;

  Ok(path)
}
