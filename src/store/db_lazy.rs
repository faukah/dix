use std::{
  fmt::{
    self,
    Display,
  },
  iter::{
    FilterMap,
    Peekable,
  },
  path::Path,
};

use eyre::{
  Context,
  Result,
  eyre,
};
use log::warn;
use ouroboros::self_referencing;
use rusqlite::{
  CachedStatement,
  MappedRows,
  Row,
};
use size::Size;

use crate::{
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
type FilterOkFunc<T> = fn(Result<T, rusqlite::Error>) -> Option<T>;

#[self_referencing]
/// Contains the SQL statement and the query resulting from it.
///
/// This is necessary since the statement is only created during
/// the query method on the Connection. The query however contains
/// a reference to it, so we can't simply return the Query
struct QueryIteratorCell<'conn, T, F>
where
  T: 'static,
  F: Fn(&rusqlite::Row) -> rusqlite::Result<T>,
{
  /// statement prepared by the sql connection
  stmt:  CachedStatement<'conn>,
  /// The actual iterator we generate from the query iterator
  ///
  /// Note that the concrete datatype is rather complicated
  /// since we currently only have a single
  /// way to deal with queries that return multiple rows and
  /// we therefore don't need to use a box.
  #[borrows(mut stmt)]
  #[not_covariant]
  inner: FilterMap<Peekable<MappedRows<'this, F>>, FilterOkFunc<T>>,
}

/// The iterator over the data resulting from a SQL query,
/// where the rows are mapped to `T`.
///
/// We ignore all rows where the conversion fails,
/// but take a look at the first row to make sure
/// the conversion is not trivially wrong.
///
/// The idea is to only use very trivial
/// conversions that will never fail
/// if the query actually returns the correct number
/// of rows.
pub struct QueryIterator<'conn, T, F>
where
  T: 'static,
  F: Fn(&rusqlite::Row) -> rusqlite::Result<T>,
{
  cell: QueryIteratorCell<'conn, T, F>,
}

impl<'conn, T, F> QueryIterator<'conn, T, F>
where
  F: Fn(&rusqlite::Row) -> rusqlite::Result<T>,
{
  /// May fail if the query itself fails or
  /// if the first row of the query result can not
  /// be mapped to `T`.
  pub fn try_new<P: rusqlite::Params>(
    stmt: CachedStatement<'conn>,
    params: P,
    map: F,
  ) -> Result<Self> {
    let cell_res = QueryIteratorCell::try_new(stmt, |stmt| {
      let inner_iter = stmt
        .query_map(params, map)
        .map(Iterator::peekable)
        .with_context(|| "Unable to perform query");

      match inner_iter {
        Ok(mut iter) => {
          #[expect(clippy::pattern_type_mismatch)]
          if let Some(Err(err)) = iter.peek() {
            return Err(eyre!("First row conversion failed: {err:?}"));
          }
          let iter_filtered = iter.filter_map(
            (|row| {
              if let Err(ref err) = row {
                log::warn!("Row conversion failed: {err:?}");
              }
              row.ok()
            }) as FilterOkFunc<T>,
          );

          Ok(iter_filtered)
        },
        Err(err) => Err(err),
      }
    });
    cell_res.map(|cell| Self { cell })
  }
}

impl<T: 'static, F> Iterator for QueryIterator<'_, T, F>
where
  F: Fn(&rusqlite::Row) -> rusqlite::Result<T>,
{
  type Item = T;
  fn next(&mut self) -> Option<Self::Item> {
    self.cell.with_inner_mut(|inner| inner.next())
  }
}

/// A lazy Nix database connection.
///
/// All returned iterators are lazy (except for the first row) and provide rows
/// as soon as they are returned by the underlying sqlite connection.
///
/// The first row is queried eagerly to catch any obvious errors in the
/// query.
///
/// # Important
/// If any errors occur in rows after the first one, these errors **will not**
/// be visible and the errors will be lost.
///
/// You may consider using an [crate::store::EagerDBConnection] instead.
#[derive(Debug)]
pub struct LazyDBConnection<'a> {
  path: &'a str,
  conn: Option<rusqlite::Connection>,
}

impl Display for LazyDBConnection<'_> {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "DBConnection({})", self.path)
  }
}

impl<'a> LazyDBConnection<'a> {
  /// Create a new connection.
  pub fn new(path: &'a str) -> LazyDBConnection<'a> {
    LazyDBConnection { path, conn: None }
  }
  /// returns a reference to the inner connection
  ///
  /// raises an error if the connection has not been established
  fn get_inner(&self) -> Result<&rusqlite::Connection> {
    self
      .conn
      .as_ref()
      .ok_or_else(|| eyre!("Attempted to use database before connecting."))
  }
  /// Executes a query that returns multiple rows and returns
  /// an iterator over them where the `map` is used to map
  /// the rows to `T`.
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
    let stmt = self.get_inner()?.prepare_cached(query)?;
    let iter = QueryIterator::try_new(stmt, [path], map)?;
    Ok(Box::new(iter))
  }
}

/// makes sure the database tries to close the connection
/// when being dropped. This is done anyways by the internal
/// connection of rusqlite, but here the error gets logged should
/// something go wrong.
impl Drop for LazyDBConnection<'_> {
  /// close the connection if it is still open
  fn drop(&mut self) {
    // try to close the connection
    if let Some(conn) = self.conn.take()
      && let Err(err) = conn.close()
    {
      warn!(
        "Tried closing database on drop but encountered error: {:?}",
        err
      )
    }
  }
}

impl<'a> StoreBackend<'a> for LazyDBConnection<'_> {
  fn connected(&self) -> bool {
    self.conn.is_some()
  }
  /// Connects to the Nix database
  ///
  /// and sets some basic settings
  fn connect(&mut self) -> Result<()> {
    self.conn = Some(db_common::default_sqlite_connection(self.path)?);
    Ok(())
  }

  /// close the inner connection to the database
  fn close(&mut self) -> Result<()> {
    db_common::default_close_inner_connection(self.path, &mut self.conn)
  }
  /// Gets the total closure size of the given store path by summing up the nar
  /// size of all dependent derivations.
  fn query_closure_size(&self, path: &Path) -> Result<Size> {
    db_common::query_closure_size(self.get_inner()?, path)
  }

  /// Gets the derivations that are directly included in the system derivation.
  ///
  /// Will not work on non-system derivations.
  fn query_system_derivations(
    &self,
    system: &Path,
  ) -> Result<Box<dyn Iterator<Item = StorePath> + '_>> {
    self.execute_row_query_with_path(
      queries::QUERY_SYSTEM_DERIVATIONS,
      system,
      |row| Ok(StorePath(row.get::<_, String>(0)?.into())),
    )
  }

  /// Gathers all derivations that the given profile path depends on.
  fn query_dependents(
    &self,
    path: &Path,
  ) -> Result<Box<dyn Iterator<Item = StorePath> + '_>> {
    self.execute_row_query_with_path(queries::QUERY_DEPENDENTS, path, |row| {
      Ok(StorePath(row.get::<_, String>(0)?.into()))
    })
  }
}
