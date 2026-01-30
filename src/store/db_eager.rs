use std::{
  fmt::{
    self,
    Display,
  },
  path::Path,
};

use anyhow::{
  Result,
  anyhow,
};
use rusqlite::Row;

use crate::{
  DerivationId,
  StorePath,
  path_to_canonical_string,
  store::{
    StoreBackend,
    db_common::{
      self,
    },
    queries,
  },
};

#[derive(Debug)]
pub struct EagerDBConnection<'a> {
  path: &'a str,
  conn: Option<rusqlite::Connection>,
}

impl Display for EagerDBConnection<'_> {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "DBConnection({})", self.path)
  }
}

impl<'a> EagerDBConnection<'a> {
  /// Create a new connection.
  pub fn new(path: &'a str) -> EagerDBConnection<'a> {
    EagerDBConnection { path, conn: None }
  }
  /// returns a reference to the inner connection
  ///
  /// raises an error if the connection has not been established
  fn get_inner(&self) -> Result<&rusqlite::Connection> {
    self
      .conn
      .as_ref()
      .ok_or_else(|| anyhow!("Attempted to use database before connecting."))
  }

  /// Executes a query that returns multiple rows and returns
  /// an iterator over them where the `map` is used to map
  /// the rows to `T`.
  ///
  /// Note that this function collects all rows before returning
  /// and raises the first error that is encountered (if any exist).
  pub(crate) fn execute_row_query_with_path<T, M>(
    &self,
    query: &str,
    path: &Path,
    map: M,
  ) -> Result<Box<dyn Iterator<Item = T> + '_>>
  where
    T: 'static,
    M: Fn(&Row) -> rusqlite::Result<T> + 'static,
  {
    let path = path_to_canonical_string(path)?;
    let mut results = Vec::new();
    let mut query = self.get_inner()?.prepare_cached(query)?;
    let queried_rows = query.query_map([path], map)?;
    for row in queried_rows {
      results.push(row?);
    }
    Ok(Box::new(results.into_iter()))
  }
}

impl<'a> StoreBackend<'a> for EagerDBConnection<'_> {
  fn connect(&mut self) -> Result<()> {
    self.conn = Some(db_common::default_sqlite_connection(self.path)?);
    Ok(())
  }

  fn connected(&self) -> bool {
    self.conn.is_some()
  }

  fn close(&mut self) -> Result<()> {
    db_common::default_close_inner_connection(self.path, &mut self.conn)
  }

  fn query_closure_size(&self, path: &std::path::Path) -> Result<size::Size> {
    db_common::query_closure_size(self.get_inner()?, path)
  }

  fn query_system_derivations(
    &self,
    system: &std::path::Path,
  ) -> Result<Box<dyn Iterator<Item = crate::StorePath> + '_>> {
    self.execute_row_query_with_path(
      queries::QUERY_SYSTEM_DERIVATIONS,
      system,
      |row| Ok(StorePath(row.get::<_, String>(0)?.into())),
    )
  }

  fn query_dependents(
    &self,
    path: &std::path::Path,
  ) -> Result<Box<dyn Iterator<Item = crate::StorePath> + '_>> {
    self.execute_row_query_with_path(queries::QUERY_DEPENDENTS, path, |row| {
      Ok(StorePath(row.get::<_, String>(0)?.into()))
    })
  }

  fn query_dependency_graph(
    &self,
    path: &std::path::Path,
  ) -> Result<
    Box<dyn Iterator<Item = (crate::DerivationId, crate::DerivationId)> + '_>,
  > {
    self.execute_row_query_with_path(
      queries::QUERY_DEPENDENCY_GRAPH,
      path,
      |row| Ok((DerivationId(row.get(0)?), DerivationId(row.get(1)?))),
    )
  }
}
