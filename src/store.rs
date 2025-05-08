use std::{
  path::{
    Path,
    PathBuf,
  },
  result,
};

use anyhow::{
  Context as _,
  Result,
};
use derive_more::Deref;
use ref_cast::RefCast;
use rusqlite::Connection;
use rustc_hash::{
  FxBuildHasher,
  FxHashMap,
};

macro_rules! path_to_str {
  ($path:ident) => {
    let $path = $path.canonicalize().with_context(|| {
      format!(
        "failed to canonicalize path '{path}'",
        path = $path.display(),
      )
    })?;

    let $path = $path.to_str().with_context(|| {
      format!(
        "failed to convert path '{path}' to valid unicode",
        path = $path.display(),
      )
    })?;
  };
}

#[derive(Deref, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DerivationId(i64);

#[expect(clippy::module_name_repetitions)]
#[derive(RefCast, Deref, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct StorePath(Path);

#[expect(clippy::module_name_repetitions)]
#[derive(Deref, Debug, Clone, PartialEq, Eq)]
pub struct StorePathBuf(PathBuf);

/// Connects to the Nix database.
pub fn connect() -> Result<Connection> {
  const DATABASE_PATH: &str = "/nix/var/nix/db/db.sqlite";

  Connection::open(DATABASE_PATH).with_context(|| {
    format!("failed to connect to Nix database at {DATABASE_PATH}")
  })
}

/// Gathers all derivations that the given store path depends on.
pub fn query_depdendents(
  connection: &mut Connection,
  path: &StorePath,
) -> Result<Vec<(DerivationId, StorePathBuf)>> {
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

  path_to_str!(path);

  let packages: result::Result<Vec<(DerivationId, StorePathBuf)>, _> =
    connection
      .prepare_cached(QUERY)?
      .query_map([path], |row| {
        Ok((
          DerivationId(row.get(0)?),
          StorePathBuf(row.get::<_, String>(1)?.into()),
        ))
      })?
      .collect();

  Ok(packages?)
}

/// Gets the total closure size of the given store path by summing up the nar
/// size of all depdendent derivations.
pub fn query_closure_size(
  connection: &mut Connection,
  path: &StorePath,
) -> Result<usize> {
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

  path_to_str!(path);

  let closure_size = connection
    .prepare_cached(QUERY)?
    .query_row([path], |row| row.get(0))?;

  Ok(closure_size)
}

/// Gathers the complete dependency graph of of the store path as an adjacency
/// list.
///
/// We might want to collect the paths in the graph directly as
/// well in the future, depending on how much we use them
/// in the operations on the graph.
pub fn query_dependency_graph(
  connection: &mut Connection,
  path: &StorePath,
) -> Result<FxHashMap<DerivationId, Vec<DerivationId>>> {
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

  path_to_str!(path);

  let mut adj =
    FxHashMap::<DerivationId, Vec<DerivationId>>::with_hasher(FxBuildHasher);

  let mut statement = connection.prepare_cached(QUERY)?;

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
