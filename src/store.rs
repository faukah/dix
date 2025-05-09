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

use crate::{
  DerivationId,
  StorePath,
};

#[derive(Deref)]
pub struct Connection(rusqlite::Connection);

/// Connects to the Nix database.
pub fn connect() -> Result<Connection> {
  const DATABASE_PATH: &str = "/nix/var/nix/db/db.sqlite";

  let inner = rusqlite::Connection::open(DATABASE_PATH).with_context(|| {
    format!("failed to connect to Nix database at {DATABASE_PATH}")
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
  /// size of all depdendent derivations.
  pub fn query_closure_size(&mut self, path: &Path) -> Result<usize> {
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
      .query_row([path], |row| row.get(0))?;

    Ok(closure_size)
  }

  /// Gathers all derivations that the given profile path depends on.
  pub fn query_depdendents(
    &mut self,
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

    let packages: result::Result<Vec<(DerivationId, StorePath)>, _> = self
      .prepare_cached(QUERY)?
      .query_map([path], |row| {
        Ok((
          DerivationId(row.get(0)?),
          StorePath(row.get::<_, String>(1)?.into()),
        ))
      })?
      .collect();

    Ok(packages?)
  }

  /// Gathers the complete dependency graph of of the store path as an adjacency
  /// list.
  ///
  /// We might want to collect the paths in the graph directly as
  /// well in the future, depending on how much we use them
  /// in the operations on the graph.
  pub fn query_dependency_graph(
    &mut self,
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
