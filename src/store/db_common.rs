use std::path::Path;

use eyre::{
  Context as _,
  Result,
  eyre,
};
use rusqlite::{
  Connection,
  OpenFlags,
};
use size::Size;

use crate::{
  path_to_canonical_string,
  store::queries,
};

pub fn default_sqlite_connection(path: &str) -> Result<Connection> {
  tracing::debug!(database_path = path, "opening sqlite connection");
  let inner = rusqlite::Connection::open_with_flags(
    path,
    OpenFlags::SQLITE_OPEN_READ_ONLY // We only run queries, safeguard against corrupting the DB.
      | OpenFlags::SQLITE_OPEN_NO_MUTEX // Part of the default flags, rusqlite takes care of locking anyways.
      | OpenFlags::SQLITE_OPEN_URI,
  )
  .with_context(|| format!("failed to connect to Nix database at {path}"))?;
  tracing::debug!(
    database_path = path,
    "sqlite connection opened successfully"
  );

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
    .with_context(|| format!("failed to cache Nix database at {path}"))?;
  Ok(inner)
}

// FIXME: why is this marked as dead code? It is used by both the lazy
// and eager backend implementation
pub fn default_close_inner_connection(
  path: &str,
  maybe_conn: &mut Option<Connection>,
) -> Result<()> {
  let conn = maybe_conn.take().ok_or_else(|| {
    eyre!("Tried to close connection to {} that does not exist", path)
  })?;
  conn.close().map_err(|(conn_old, err)| {
    *maybe_conn = Some(conn_old);
    eyre::Report::from(err).wrap_err("failed to close Nix database")
  })
}

pub fn query_closure_size(conn: &Connection, path: &Path) -> Result<Size> {
  tracing::trace!(path = %path.display(), "querying closure size");
  let path = path_to_canonical_string(path)?;

  let closure_size = conn
    .prepare_cached(queries::QUERY_CLOSURE_SIZE)?
    .query_row([path], |row| Ok(Size::from_bytes(row.get::<_, i64>(0)?)))?;

  Ok(closure_size)
}
