#![allow(clippy::mem_forget)]

use std::{
  iter::{
    FilterMap,
    Iterator,
    Peekable,
  },
  path::Path,
};

use anyhow::{
  Context as _,
  Result,
  anyhow,
};
use log::warn;
use ouroboros::self_referencing;
use rusqlite::{
  CachedStatement,
  MappedRows,
  OpenFlags,
  Row,
};
use size::Size;

use crate::{
  DerivationId,
  StorePath,
};
/// the normal database connection
pub const DATABASE_PATH: &str = "file:/nix/var/nix/db/db.sqlite";
/// a backup database connection that can access the database
/// even in a read-only environment
///
/// might product incorrect results as the connection is not guaranteed
/// to be the only one accessing the database. (There might be e.g. a
/// nixos-rebuild modifying the database)
pub const DATABASE_PATH_IMMUTABLE: &str =
  "file:/nix/var/nix/db/db.sqlite?immutable=1";

/// defines an interface for interacting with a Nix database.
///
/// This allows us to construct a frontend that can fall back
/// to e.g. shell commands should something go wrong.
pub(crate) trait StoreFrontend {
  fn connect(&mut self) -> Result<()>;
  fn close(&mut self) -> Result<()>;
  fn query_closure_size(&self, path: &StorePath) -> Result<Size>;
  fn query_system_derivations(
    &self,
    system: &Path,
  ) -> Result<impl Iterator<Item = (DerivationId, StorePath)>>;
  fn query_dependents(
    &self,
    path: &Path,
  ) -> Result<impl Iterator<Item = (DerivationId, StorePath)>>;
  fn query_dependency_graph(
    &self,
    path: &StorePath,
  ) -> Result<impl Iterator<Item = (DerivationId, DerivationId)>>;
}

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
  /// note that the concrete datatype is rather complicated,
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
            return Err(anyhow!("First row conversion failed: {err:?}"));
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

/// A Nix database connection.
pub struct DBConnection<'a> {
  path: &'a str,
  conn: Option<rusqlite::Connection>,
}

impl<'a> DBConnection<'a> {
  /// create a new connection
  pub fn new(path: &'a str) -> DBConnection<'a> {
    DBConnection { path, conn: None }
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
  /// returns a mutable reference to the inner connection
  ///
  /// raises an error if the connection has not been established
  fn get_inner_mut(&mut self) -> Result<&mut rusqlite::Connection> {
    self
      .conn
      .as_mut()
      .ok_or_else(|| anyhow!("Attempted to use database before connecting."))
  }
  /// Executes a query that returns multiple rows and returns
  /// an iterator over them where the `map` is used to map
  /// the rows to `T`.
  pub(crate) fn execute_row_query_with_path<T, M>(
    &self,
    query: &str,
    path: &Path,
    map: M,
  ) -> Result<impl Iterator<Item = T>>
  where
    T: 'static,
    M: Fn(&Row) -> rusqlite::Result<T>,
  {
    let path = path_to_canonical_string(path)?;
    let stmt = self.get_inner()?.prepare_cached(query)?;
    QueryIterator::try_new(stmt, [path], map)
  }
}

// TODO: is this a good idea?
impl Drop for DBConnection<'_> {
  /// close the connection when the DB is closed
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

impl StoreFrontend for DBConnection<'_> {
  /// Connects to the Nix database
  ///
  /// and sets some basic settings
  fn connect(&mut self) -> Result<()> {
    let inner = rusqlite::Connection::open_with_flags(
      self.path,
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
        format!("failed to cache Nix database at {}", self.path)
      })?;

    self.conn = Some(inner);
    Ok(())
  }

  /// close the inner connection to the database
  fn close(&mut self) -> Result<()> {
    // TODO: we might want to consider dropping self (and follow how the inner
    // conn works)
    let conn = self.conn.take().ok_or_else(|| {
      anyhow!(
        "Tried to close connection to {} that does not exist",
        self.path
      )
    })?;
    conn.close().map_err(|(_conn, err)| {
      anyhow::Error::from(err).context("failed to close Nix database")
    })
  }

  /// Gets the total closure size of the given store path by summing up the nar
  /// size of all dependent derivations.
  fn query_closure_size(&self, path: &StorePath) -> Result<Size> {
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
      .get_inner()?
      .prepare_cached(QUERY)?
      .query_row([path], |row| Ok(Size::from_bytes(row.get::<_, i64>(0)?)))?;

    Ok(closure_size)
  }

  /// Gets the derivations that are directly included in the system derivation.
  ///
  /// Will not work on non-system derivations.
  fn query_system_derivations(
    &self,
    system: &Path,
  ) -> Result<impl Iterator<Item = (DerivationId, StorePath)>> {
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

    self.execute_row_query_with_path(QUERY, system, |row| {
      Ok((
        DerivationId(row.get(0)?),
        StorePath(row.get::<_, String>(1)?.into()),
      ))
    })
  }

  /// Gathers all derivations that the given profile path depends on.
  fn query_dependents(
    &self,
    path: &Path,
  ) -> Result<impl Iterator<Item = (DerivationId, StorePath)>> {
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

    self.execute_row_query_with_path(QUERY, path, |row| {
      Ok((
        DerivationId(row.get(0)?),
        StorePath(row.get::<_, String>(1)?.into()),
      ))
    })
  }

  /// Returns all edges of the dependency graph.
  ///
  /// You might want to build an adjacency list from the resulting
  /// edges.
  #[expect(dead_code)]
  fn query_dependency_graph(
    &self,
    path: &StorePath,
  ) -> Result<impl Iterator<Item = (DerivationId, DerivationId)>> {
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

    self.execute_row_query_with_path(QUERY, path, |row| {
      Ok((DerivationId(row.get(0)?), DerivationId(row.get(1)?)))
    })
  }
}
