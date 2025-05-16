use std::{
  collections::HashMap,
  path::Path,
  result,
};

use anyhow::{
  Context as _,
  Result,
  anyhow,
};
use derive_more::Deref;
use rusqlite::OpenFlags;
use size::Size;

use crate::{
  DerivationId,
  StorePath,
};

#[derive(Deref)]
pub struct Connection(rusqlite::Connection);

/// Connects to the Nix database
///
/// and sets some basic settings
///
/// # Errors
///
/// Returns an error if the functions fails to connect to the database located
/// at `/nix/var/nix/db/db.sqlite`.
pub fn connect() -> Result<Connection> {
  const DATABASE_PATH: &str = "/nix/var/nix/db/db.sqlite";

  let inner = rusqlite::Connection::open_with_flags(
    DATABASE_PATH,
    OpenFlags::SQLITE_OPEN_READ_ONLY // We only run queries, safeguard against corrupting the DB.
      | OpenFlags::SQLITE_OPEN_NO_MUTEX // Part of the default flags, rusqlite takes care of locking anyways.
      | OpenFlags::SQLITE_OPEN_URI,
  )
  .with_context(|| {
    format!("failed to connect to Nix database at {DATABASE_PATH}")
  })?;

  // Perform a batched query to set some settings using PRAGMA
  // the main performance bottleneck when dix was run before
  // was that the database file has to be brought from disk into
  // memory.
  //
  // We read a large part of the DB anyways in each query,
  // so it makes sense to set aside a large region of memory-mapped
  // I/O prevent incurring page faults which can be done using
  // `mmap_size`.
  //
  // This made a performance difference of about 500ms (but only
  // when it was first run for a long time!).
  //
  // The file pages of the store can be evicted from main memory
  // using:
  //
  // ```bash
  // dd of=/nix/var/nix/db/db.sqlite oflag=nocache conv=notrunc,fdatasync count=0
  // ```
  //
  // If you want to test this. Source: <https://unix.stackexchange.com/questions/36907/drop-a-specific-file-from-the-linux-filesystem-cache>.
  //
  // Documentation about the settings can be found here: <https://www.sqlite.org/pragma.html>
  //
  // [0]: 256MB, enough to fit the whole DB (at least on my system - Dragyx).
  // [1]: Always store temporary tables in memory.
  inner
    .execute_batch(
      "
        PRAGMA mmap_size=268435456; -- See [0].
        PRAGMA temp_store=2; -- See [1].
        PRAGMA query_only;
      ",
    )
    .with_context(|| {
      format!("failed to cache Nix database at {DATABASE_PATH}")
    })?;

  Ok(Connection(inner))
}

fn path_to_canonical_string(path: &Path) -> Result<String> {
  let path = path.canonicalize().with_context(|| {
    format!(
      "failed to canonicalize path '{path}'",
      path = path.display(),
    )
  })?;

  let path = path.into_os_string().into_string().map_err(|path| {
    anyhow!(
      "failed to convert path '{path}' to valid unicode",
      path = Path::new(&*path).display(), /* TODO: use .display() directly
                                           * after Rust 1.87.0 in flake. */
    )
  })?;

  Ok(path)
}

impl Connection {
  /// Gets the total closure size of the given store path by summing up the nar
  /// size of all dependent derivations.
  #[expect(clippy::missing_errors_doc)]
  pub fn query_closure_size(&self, path: &Path) -> Result<Size> {
    const QUERY: &str = "
      WITH RECURSIVE
        graph(p) AS (
          SELECT id 
          FROM ValidPaths
          WHERE path = ?
        UNION
          SELECT reference FROM Refs
          JOIN graph ON referrer = p
        )
      SELECT SUM(narSize) as sum from graph
      JOIN ValidPaths ON p = id;
    ";

    let path = path_to_canonical_string(path)?;

    let closure_size = self
      .prepare_cached(QUERY)?
      .query_row([path], |row| Ok(Size::from_bytes(row.get::<_, i64>(0)?)))?;

    Ok(closure_size)
  }

  /// Gets the derivations that are directly included in the system derivation.
  ///
  /// Will not work on non-system derivations.
  #[expect(clippy::missing_errors_doc)]
  pub fn query_system_derivations(
    &self,
    system: &Path,
  ) -> Result<Vec<(DerivationId, StorePath)>> {
    const QUERY: &str = "
      WITH
        systemderiv AS (
          SELECT id FROM ValidPaths
          WHERE path = ?
        ),
        systempath AS (
          SELECT reference as id FROM systemderiv sd
          JOIN Refs ON sd.id = referrer
          JOIN ValidPaths vp ON reference = vp.id
          WHERE (vp.path LIKE '%-system-path')
        ),
        pkgs AS (
            SELECT reference as id FROM Refs
            JOIN systempath ON referrer = id
        )
      SELECT pkgs.id, path FROM pkgs
      JOIN ValidPaths vp ON vp.id = pkgs.id;
    ";

    let path = path_to_canonical_string(system)?;

    let derivations: result::Result<Vec<(DerivationId, StorePath)>, _> = self
      .prepare_cached(QUERY)?
      .query_map([path], |row| {
        Ok((
          DerivationId(row.get(0)?),
          StorePath(row.get::<_, String>(1)?.into()),
        ))
      })?
      .collect();

    Ok(derivations?)
  }

  /// Gathers all derivations that the given profile path depends on.
  #[expect(clippy::missing_errors_doc)]
  pub fn query_dependents(
    &self,
    path: &Path,
  ) -> Result<Vec<(DerivationId, StorePath)>> {
    const QUERY: &str = "
      WITH RECURSIVE
        graph(p) AS (
          SELECT id 
          FROM ValidPaths
          WHERE path = ?
        UNION
          SELECT reference FROM Refs
          JOIN graph ON referrer = p
        )
      SELECT id, path from graph
      JOIN ValidPaths ON id = p;
    ";

    let path = path_to_canonical_string(path)?;

    let derivations: result::Result<Vec<(DerivationId, StorePath)>, _> = self
      .prepare_cached(QUERY)?
      .query_map([path], |row| {
        Ok((
          DerivationId(row.get(0)?),
          StorePath(row.get::<_, String>(1)?.into()),
        ))
      })?
      .collect();

    Ok(derivations?)
  }

  /// Gathers the complete dependency graph of of the store path as an adjacency
  /// list.
  ///
  /// We might want to collect the paths in the graph directly as
  /// well in the future, depending on how much we use them
  /// in the operations on the graph.
  #[expect(clippy::missing_errors_doc)]
  pub fn query_dependency_graph(
    &self,
    path: &StorePath,
  ) -> Result<HashMap<DerivationId, Vec<DerivationId>>> {
    const QUERY: &str = "
      WITH RECURSIVE
        graph(p, c) AS (
          SELECT id as par, reference as chd 
          FROM ValidPaths
          JOIN Refs ON referrer = id
          WHERE path = ?
        UNION
          SELECT referrer as par, reference as chd FROM Refs
          JOIN graph ON referrer = c
        )
      SELECT p, c from graph;
    ";

    let path = path_to_canonical_string(path)?;

    let mut adj = HashMap::<DerivationId, Vec<DerivationId>>::new();

    let mut statement = self.prepare_cached(QUERY)?;

    let edges = statement.query_map([path], |row| {
      Ok((DerivationId(row.get(0)?), DerivationId(row.get(1)?)))
    })?;

    for row in edges {
      let (from, to) = row?;

      adj.entry(from).or_default().push(to);
      adj.entry(to).or_default();
    }

    Ok(adj)
  }
}
