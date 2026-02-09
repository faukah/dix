// Test utilities and infrastructure for database testing.
//
// This module provides utilities to create temporary SQLite databases
// with the Nix store schema for testing purposes.

use std::{
  fs,
  path::{
    Path,
    PathBuf,
  },
};

use eyre::Result;
use rusqlite::Connection;
use tempfile::TempDir;

/// Test database builder for creating temporary `SQLite` databases
/// with the Nix store schema.
pub struct TestDbBuilder {
  temp_dir: TempDir,
  db_path:  PathBuf,
}

impl TestDbBuilder {
  /// Creates a new test database builder.
  ///
  /// This creates a temporary directory and initializes an `SQLite` database
  /// with the Nix store schema.
  pub fn new() -> Result<Self> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.db");

    let builder = Self { temp_dir, db_path };

    builder.init_schema()?;

    Ok(builder)
  }

  /// Returns the path to the database file.
  pub fn db_path(&self) -> &Path {
    &self.db_path
  }

  /// Returns the path to the temporary directory.
  pub fn temp_dir(&self) -> &TempDir {
    &self.temp_dir
  }

  /// Returns the actual filesystem path for a given fixture path.
  ///
  /// This converts a path like `/nix/store/xxx-name` to the actual
  /// path in the temp directory like `/tmp/.../xxx-name`.
  pub fn resolve_fixture_path(&self, fixture_path: &str) -> PathBuf {
    let relative_path = fixture_path
      .strip_prefix("/nix/store/")
      .unwrap_or(fixture_path);

    self.temp_dir.path().join(relative_path)
  }

  /// Opens a read-only connection to the database with test flags.
  pub fn open_readonly(&self) -> Result<Connection> {
    let conn = rusqlite::Connection::open_with_flags(
      &self.db_path,
      rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
        | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;

    conn.execute_batch(
      "
        PRAGMA mmap_size=268435456;
        PRAGMA temp_store=2;
        PRAGMA query_only;
      ",
    )?;

    Ok(conn)
  }

  /// Opens a read-write connection to the database.
  pub fn open_readwrite(&self) -> Result<Connection> {
    let conn = rusqlite::Connection::open(&self.db_path)?;
    Ok(conn)
  }

  /// Initializes the Nix store database schema.
  fn init_schema(&self) -> Result<()> {
    let conn = self.open_readwrite()?;

    conn.execute_batch(
      "
        CREATE TABLE IF NOT EXISTS ValidPaths (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          path TEXT NOT NULL UNIQUE,
          hash TEXT NOT NULL,
          registrationTime INTEGER NOT NULL,
          deriver TEXT,
          narSize INTEGER NOT NULL,
          ultimate INTEGER,
          sigs TEXT,
          ca TEXT
        );

        CREATE TABLE IF NOT EXISTS Refs (
          referrer INTEGER NOT NULL,
          reference INTEGER NOT NULL,
          PRIMARY KEY (referrer, reference),
          FOREIGN KEY (referrer) REFERENCES ValidPaths(id),
          FOREIGN KEY (reference) REFERENCES ValidPaths(id)
        );

        CREATE INDEX IF NOT EXISTS IndexRefs ON Refs(referrer);
        CREATE INDEX IF NOT EXISTS IndexPath ON ValidPaths(path);
      ",
    )?;

    conn.close().map_err(|(_, err)| err)?;

    Ok(())
  }

  /// Adds a valid path to the database.
  ///
  /// Returns the ID of the newly created entry.
  /// Also creates the path as a directory in the temp directory so it can be
  /// canonicalized.
  pub fn add_valid_path(&self, path: &str, nar_size: i64) -> Result<i64> {
    // Create the directory on the filesystem so it can be canonicalized
    let fs_path = self.resolve_fixture_path(path);
    fs::create_dir_all(&fs_path)?;

    // Store the actual filesystem path that can be canonicalized
    let canonical_path = fs_path.canonicalize()?;
    let path_str = canonical_path.to_string_lossy();

    let conn = self.open_readwrite()?;
    let mut stmt = conn.prepare(
      "INSERT INTO ValidPaths (path, hash, registrationTime, narSize) VALUES \
       (?1, ?2, ?3, ?4) RETURNING id",
    )?;

    let id = stmt.query_row(
      [&path_str, "test-hash", "1234567890", &nar_size.to_string()],
      |row| row.get::<_, i64>(0),
    )?;
    drop(stmt);
    conn.close().map_err(|(_, err)| err)?;

    Ok(id)
  }

  /// Adds a reference relationship between two valid paths.
  ///
  /// Both paths must already exist in the `ValidPaths` table.
  pub fn add_reference(
    &self,
    referrer_id: i64,
    reference_id: i64,
  ) -> Result<()> {
    let conn = self.open_readwrite()?;
    let mut stmt =
      conn.prepare("INSERT INTO Refs (referrer, reference) VALUES (?1, ?2)")?;
    stmt.execute([referrer_id, reference_id])?;
    drop(stmt);
    conn.close().map_err(|(_, err)| err)?;

    Ok(())
  }

  /// Looks up the ID for a path.
  pub fn get_path_id(&self, path: &str) -> Result<i64> {
    let conn = self.open_readwrite()?;
    let mut stmt = conn.prepare("SELECT id FROM ValidPaths WHERE path = ?1")?;
    let id = stmt.query_row([path], |row| row.get::<_, i64>(0))?;
    drop(stmt);
    conn.close().map_err(|(_, err)| err)?;

    Ok(id)
  }

  /// Creates a complete closure with paths and references.
  ///
  /// This is a convenience method for creating a test closure.
  /// `paths` is a list of (path, `nar_size`) tuples.
  /// `refs` is a list of (`referrer_path`, `reference_path`) tuples.
  pub fn create_closure(
    &self,
    paths: Vec<(&str, i64)>,
    refs: Vec<(&str, &str)>,
  ) -> Result<Closure> {
    let mut path_ids = std::collections::HashMap::new();

    // Add all paths
    for (path, nar_size) in paths {
      let id = self.add_valid_path(path, nar_size)?;
      path_ids.insert(path.to_string(), id);
    }

    // Add all references
    for (referrer, reference) in refs {
      let referrer_id = *path_ids
        .entry(referrer.to_string())
        .or_insert_with(|| self.get_path_id(referrer).unwrap());

      let reference_id = *path_ids
        .entry(reference.to_string())
        .or_insert_with(|| self.get_path_id(reference).unwrap());

      self.add_reference(referrer_id, reference_id)?;
    }

    Ok(Closure { path_ids })
  }
}

/// Represents a closure of related paths in a test database.
pub struct Closure {
  path_ids: std::collections::HashMap<String, i64>,
}

impl Closure {
  /// Returns the ID for a path in this closure.
  pub fn get_id(&self, path: &str) -> Option<i64> {
    self.path_ids.get(path).copied()
  }
}

/// Standard test fixtures for Nix store paths.
pub mod fixtures {

  /// Returns a standard store path prefix.
  pub fn store_prefix() -> &'static str {
    "/nix/store/00000000000000000000000000000000-"
  }

  /// Creates a full store path from a package name.
  pub fn store_path(name: &str) -> String {
    format!("{}{}", store_prefix(), name)
  }

  /// Creates a system derivation path.
  pub fn system_path(name: &str) -> String {
    format!("{}-system", store_path(name))
  }

  /// Creates a system-path derivation.
  pub fn system_path_derivation(name: &str) -> String {
    format!("{}-system-path", store_path(name))
  }
}

/// Creates a test database with a simple closure.
///
/// This creates:
/// - A root path with narSize 100
/// - Two dependencies with narSize 50 each
/// - A reference from root to each dependency
pub fn create_simple_test_db() -> Result<TestDbBuilder> {
  let db = TestDbBuilder::new()?;

  let root = fixtures::store_path("root-package");
  let dep1 = fixtures::store_path("dependency-1.0");
  let dep2 = fixtures::store_path("dependency-2.0");

  db.create_closure(vec![(&root, 100), (&dep1, 50), (&dep2, 50)], vec![
    (&root, &dep1),
    (&root, &dep2),
  ])?;

  Ok(db)
}

/// Creates a test database simulating a NixOS system closure.
///
/// This creates a realistic structure with:
/// - A system derivation
/// - A system-path containing packages
/// - Multiple packages with dependencies
pub fn create_system_test_db() -> Result<TestDbBuilder> {
  let db = TestDbBuilder::new()?;

  // System-level paths
  let system = fixtures::system_path("nixos-23.11");
  let system_path = fixtures::system_path_derivation("nixos-23.11");

  // Package paths
  let glibc = fixtures::store_path("glibc-2.38");
  let gcc = fixtures::store_path("gcc-12.3.0");
  let bash = fixtures::store_path("bash-5.2.15");
  let coreutils = fixtures::store_path("coreutils-9.3");
  let systemd = fixtures::store_path("systemd-254.6");

  // Create all paths with realistic sizes
  db.create_closure(
    vec![
      (&system, 0),           // System derivation has no size
      (&system_path, 1000),   // System path aggregates
      (&glibc, 50000000),     // ~50MB
      (&gcc, 150000000),      // ~150MB
      (&bash, 5000000),       // ~5MB
      (&coreutils, 10000000), // ~10MB
      (&systemd, 80000000),   // ~80MB
    ],
    vec![
      // System references system-path
      (&system, &system_path),
      // System-path references packages
      (&system_path, &bash),
      (&system_path, &coreutils),
      (&system_path, &systemd),
      // Package dependencies
      (&gcc, &glibc),
      (&bash, &glibc),
      (&coreutils, &glibc),
      (&systemd, &glibc),
    ],
  )?;

  Ok(db)
}

/// Creates a test database with a deep dependency chain.
///
/// This creates:
/// - A chain of n packages where each depends on the next
/// - Each with incrementing sizes
pub fn create_chain_test_db(n: usize) -> Result<TestDbBuilder> {
  let db = TestDbBuilder::new()?;

  let mut paths = Vec::new();
  let mut refs = Vec::new();

  for i in 0..n {
    let path = fixtures::store_path(&format!("package-{i}"));
    let size = (i as i64 + 1) * 1000;
    paths.push((path.clone(), size));

    if i > 0 {
      let prev_path = fixtures::store_path(&format!("package-{}", i - 1));
      refs.push((prev_path.clone(), path.clone()));
    }
  }

  // Convert to references for create_closure
  let paths_ref: Vec<(&str, i64)> =
    paths.iter().map(|(p, s)| (p.as_str(), *s)).collect();
  let refs_ref: Vec<(&str, &str)> =
    refs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
  db.create_closure(paths_ref, refs_ref)?;

  Ok(db)
}

/// Creates a test database with a diamond dependency pattern.
///
///    A
///   / \
///  B   C
///   \ /
///    D
pub fn create_diamond_test_db() -> Result<TestDbBuilder> {
  let db = TestDbBuilder::new()?;

  let a = fixtures::store_path("package-a");
  let b = fixtures::store_path("package-b");
  let c = fixtures::store_path("package-c");
  let d = fixtures::store_path("package-d");

  db.create_closure(vec![(&a, 1000), (&b, 500), (&c, 500), (&d, 250)], vec![
    (&a, &b),
    (&a, &c),
    (&b, &d),
    (&c, &d),
  ])?;

  Ok(db)
}

#[cfg(test)]
mod db_eager_tests {
  use size::Size;

  use crate::store::{
    StoreBackend,
    db_eager::EagerDBConnection,
    test_utils::{
      create_diamond_test_db,
      create_simple_test_db,
      create_system_test_db,
      fixtures,
    },
  };

  #[test]
  fn test_eager_query_closure_size() {
    let db = create_simple_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = EagerDBConnection::new(&db_path_str);
    conn.connect().unwrap();
    assert!(conn.connected());

    let root_fixture = fixtures::store_path("root-package");
    let root = db.resolve_fixture_path(&root_fixture);
    let result = conn.query_closure_size(&root);
    assert!(result.is_ok());

    // The closure should include root (100) + dep1 (50) + dep2 (50) = 200
    let size = result.unwrap();
    assert_eq!(size, Size::from_bytes(200));

    conn.close().unwrap();
    assert!(!conn.connected());
  }

  #[test]
  fn test_eager_query_dependents() {
    let db = create_diamond_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = EagerDBConnection::new(&db_path_str);
    conn.connect().unwrap();

    let a_fixture = fixtures::store_path("package-a");
    let a = db.resolve_fixture_path(&a_fixture);
    let result = conn.query_dependents(&a);
    assert!(result.is_ok());

    // In a diamond pattern (A -> B, C -> D), A's dependents are A, B, C, D
    let dependents: Vec<_> = result.unwrap().collect();
    assert_eq!(dependents.len(), 4);

    conn.close().unwrap();
  }

  #[test]
  fn test_eager_query_system_derivations() {
    let db = create_system_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = EagerDBConnection::new(&db_path_str);
    conn.connect().unwrap();

    let system_fixture = fixtures::system_path("nixos-23.11");
    let system = db.resolve_fixture_path(&system_fixture);
    let result = conn.query_system_derivations(&system);
    assert!(result.is_ok());

    // System derivations should be the packages referenced by system-path
    let derivations: Vec<_> = result.unwrap().collect();
    assert!(!derivations.is_empty());

    conn.close().unwrap();
  }
}

#[cfg(test)]
mod db_lazy_tests {
  use size::Size;

  use crate::store::{
    StoreBackend,
    db_lazy::LazyDBConnection,
    test_utils::{
      create_diamond_test_db,
      create_simple_test_db,
      create_system_test_db,
      fixtures,
    },
  };

  #[test]
  fn test_lazy_query_closure_size() {
    let db = create_simple_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = LazyDBConnection::new(&db_path_str);
    conn.connect().unwrap();
    assert!(conn.connected());

    let root_fixture = fixtures::store_path("root-package");
    let root = db.resolve_fixture_path(&root_fixture);
    let result = conn.query_closure_size(&root);
    assert!(result.is_ok());

    // The closure should include root (100) + dep1 (50) + dep2 (50) = 200
    let size = result.unwrap();
    assert_eq!(size, Size::from_bytes(200));

    conn.close().unwrap();
    assert!(!conn.connected());
  }

  #[test]
  fn test_lazy_query_dependents() {
    let db = create_diamond_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = LazyDBConnection::new(&db_path_str);
    conn.connect().unwrap();

    let a_fixture = fixtures::store_path("package-a");
    let a = db.resolve_fixture_path(&a_fixture);
    let result = conn.query_dependents(&a);
    assert!(result.is_ok());

    // In a diamond pattern (A -> B, C -> D), A's dependents are A, B, C, D
    let dependents: Vec<_> = result.unwrap().collect();
    assert_eq!(dependents.len(), 4);

    conn.close().unwrap();
  }

  #[test]
  fn test_lazy_query_system_derivations() {
    let db = create_system_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = LazyDBConnection::new(&db_path_str);
    conn.connect().unwrap();

    let system_fixture = fixtures::system_path("nixos-23.11");
    let system = db.resolve_fixture_path(&system_fixture);
    let result = conn.query_system_derivations(&system);
    assert!(result.is_ok());

    // System derivations should be the packages referenced by system-path
    let derivations: Vec<_> = result.unwrap().collect();
    assert!(!derivations.is_empty());

    conn.close().unwrap();
  }

  #[test]
  fn test_lazy_connection_auto_close_on_drop() {
    let db = create_simple_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    {
      let mut conn = LazyDBConnection::new(&db_path_str);
      conn.connect().unwrap();
      assert!(conn.connected());
      // Connection will be dropped here
    }

    // After drop, the connection should be closed
    // (We can't directly test this, but we verified it doesn't panic)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_db_builder_creation() {
    let db = TestDbBuilder::new().unwrap();
    assert!(db.db_path().exists());
  }

  #[test]
  fn test_add_valid_path() {
    let db = TestDbBuilder::new().unwrap();
    let path = fixtures::store_path("test-package");
    let id = db.add_valid_path(&path, 1000).unwrap();
    assert!(id > 0);
  }

  #[test]
  fn test_add_reference() {
    let db = TestDbBuilder::new().unwrap();

    let parent = fixtures::store_path("parent");
    let child = fixtures::store_path("child");

    let parent_id = db.add_valid_path(&parent, 1000).unwrap();
    let child_id = db.add_valid_path(&child, 500).unwrap();

    db.add_reference(parent_id, child_id).unwrap();

    // Get the actual filesystem path that was stored in the database
    let parent_fs_path = db.resolve_fixture_path(&parent);
    let retrieved_id =
      db.get_path_id(&parent_fs_path.to_string_lossy()).unwrap();
    assert_eq!(retrieved_id, parent_id);
  }

  #[test]
  fn test_create_closure() {
    let db = TestDbBuilder::new().unwrap();

    let root = fixtures::store_path("root");
    let dep = fixtures::store_path("dep");

    let closure = db
      .create_closure(vec![(&root, 100), (&dep, 50)], vec![(&root, &dep)])
      .unwrap();

    assert!(closure.get_id(&root).is_some());
    assert!(closure.get_id(&dep).is_some());
  }

  #[test]
  fn test_simple_test_db() {
    let db = create_simple_test_db().unwrap();
    let conn = db.open_readonly().unwrap();

    // Verify schema exists
    let count: i64 = conn
      .query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND \
         name='ValidPaths'",
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(count, 1);

    // Verify data exists
    let path_count: i64 = conn
      .query_row("SELECT COUNT(*) FROM ValidPaths", [], |row| row.get(0))
      .unwrap();
    assert_eq!(path_count, 3);
  }

  #[test]
  fn test_system_test_db() {
    let db = create_system_test_db().unwrap();
    let conn = db.open_readonly().unwrap();

    let path_count: i64 = conn
      .query_row("SELECT COUNT(*) FROM ValidPaths", [], |row| row.get(0))
      .unwrap();
    assert!(
      path_count >= 7,
      "Expected at least 7 paths, got {path_count}"
    );
  }

  #[test]
  fn test_chain_test_db() {
    let db = create_chain_test_db(5).unwrap();
    let conn = db.open_readonly().unwrap();

    let path_count: i64 = conn
      .query_row("SELECT COUNT(*) FROM ValidPaths", [], |row| row.get(0))
      .unwrap();
    assert_eq!(path_count, 5);

    let ref_count: i64 = conn
      .query_row("SELECT COUNT(*) FROM Refs", [], |row| row.get(0))
      .unwrap();
    assert_eq!(ref_count, 4);
  }

  #[test]
  fn test_diamond_test_db() {
    let db = create_diamond_test_db().unwrap();
    let conn = db.open_readonly().unwrap();

    let path_count: i64 = conn
      .query_row("SELECT COUNT(*) FROM ValidPaths", [], |row| row.get(0))
      .unwrap();
    assert_eq!(path_count, 4);

    let ref_count: i64 = conn
      .query_row("SELECT COUNT(*) FROM Refs", [], |row| row.get(0))
      .unwrap();
    assert_eq!(ref_count, 4);
  }
}

/// Edge case test fixtures and scenarios.
pub mod edge_cases {
  use super::*;

  /// Creates an empty test database with no paths.
  pub fn create_empty_test_db() -> Result<TestDbBuilder> {
    TestDbBuilder::new()
  }

  /// Creates a test database with a single isolated path.
  pub fn create_isolated_path_test_db() -> Result<TestDbBuilder> {
    let db = TestDbBuilder::new()?;
    let path = fixtures::store_path("isolated-package");
    db.add_valid_path(&path, 1000)?;
    Ok(db)
  }

  /// Creates a test database with a self-referencing path.
  /// This tests if recursive CTEs handle cycles correctly.
  pub fn create_self_reference_test_db() -> Result<TestDbBuilder> {
    let db = TestDbBuilder::new()?;
    let path = fixtures::store_path("self-referential");
    let id = db.add_valid_path(&path, 500)?;
    // Add a reference to itself
    db.add_reference(id, id)?;
    Ok(db)
  }

  /// Creates a test database with a circular dependency.
  /// A -> B -> C -> A
  pub fn create_circular_test_db() -> Result<TestDbBuilder> {
    let db = TestDbBuilder::new()?;

    let a = fixtures::store_path("circular-a");
    let b = fixtures::store_path("circular-b");
    let c = fixtures::store_path("circular-c");

    let id_a = db.add_valid_path(&a, 100)?;
    let id_b = db.add_valid_path(&b, 100)?;
    let id_c = db.add_valid_path(&c, 100)?;

    // Create cycle: A -> B -> C -> A
    db.add_reference(id_a, id_b)?;
    db.add_reference(id_b, id_c)?;
    db.add_reference(id_c, id_a)?;

    Ok(db)
  }

  /// Creates a test database with disconnected components.
  /// Multiple independent closures that don't reference each other.
  pub fn create_disconnected_test_db() -> Result<TestDbBuilder> {
    let db = TestDbBuilder::new()?;

    // First component
    let a1 = fixtures::store_path("component1-a");
    let b1 = fixtures::store_path("component1-b");

    // Second component
    let a2 = fixtures::store_path("component2-a");
    let b2 = fixtures::store_path("component2-b");

    let id_a1 = db.add_valid_path(&a1, 100)?;
    let id_b1 = db.add_valid_path(&b1, 50)?;
    let id_a2 = db.add_valid_path(&a2, 100)?;
    let id_b2 = db.add_valid_path(&b2, 50)?;

    // References within each component
    db.add_reference(id_a1, id_b1)?;
    db.add_reference(id_a2, id_b2)?;

    Ok(db)
  }

  /// Creates a test database with a very wide dependency tree.
  /// One root with many direct children.
  pub fn create_wide_tree_test_db(n_children: usize) -> Result<TestDbBuilder> {
    let db = TestDbBuilder::new()?;

    let root = fixtures::store_path("wide-root");
    let root_id = db.add_valid_path(&root, 1000)?;

    for i in 0..n_children {
      let child = fixtures::store_path(&format!("wide-child-{i}"));
      let child_id = db.add_valid_path(&child, 50)?;
      db.add_reference(root_id, child_id)?;
    }

    Ok(db)
  }

  /// Creates a test database with a deeply nested chain.
  /// Tests recursion depth limits.
  pub fn create_deep_chain_test_db(depth: usize) -> Result<TestDbBuilder> {
    let db = TestDbBuilder::new()?;

    if depth == 0 {
      return Ok(db);
    }

    let mut prev_id =
      db.add_valid_path(&fixtures::store_path("deep-0"), 100)?;

    for i in 1..depth {
      let path = fixtures::store_path(&format!("deep-{i}"));
      let id = db.add_valid_path(&path, 100)?;

      db.add_reference(prev_id, id)?;
      prev_id = id;
    }

    Ok(db)
  }

  /// Creates a test database with paths containing special characters.
  /// Tests string handling and SQL injection safety.
  pub fn create_special_chars_test_db() -> Result<TestDbBuilder> {
    let db = TestDbBuilder::new()?;

    // Paths with special characters
    let special1 = fixtures::store_path("package-with-dashes-and-dots-1.2.3");
    let special2 = fixtures::store_path("package_with_underscores");
    let special3 = fixtures::store_path("UPPERCASE-PACKAGE");
    let special4 = fixtures::store_path("package'with'quotes");

    let id1 = db.add_valid_path(&special1, 100)?;
    let id2 = db.add_valid_path(&special2, 50)?;
    let _id3 = db.add_valid_path(&special3, 75)?;
    let _id4 = db.add_valid_path(&special4, 25)?;

    db.add_reference(id1, id2)?;

    Ok(db)
  }
}

#[cfg(test)]
mod edge_case_tests {
  use size::Size;

  use crate::store::{
    StoreBackend,
    db_eager::EagerDBConnection,
    db_lazy::LazyDBConnection,
    test_utils::edge_cases,
  };

  #[test]
  fn test_empty_database() {
    let db = edge_cases::create_empty_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = EagerDBConnection::new(&db_path_str);
    conn.connect().unwrap();

    // Query on non-existent path should fail gracefully
    let fake_path = db.temp_dir().path().join("nonexistent");
    let result = conn.query_closure_size(&fake_path);
    assert!(result.is_err());

    conn.close().unwrap();
  }

  #[test]
  fn test_isolated_path() {
    let db = edge_cases::create_isolated_path_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = EagerDBConnection::new(&db_path_str);
    conn.connect().unwrap();

    // Get the actual filesystem path
    let path_fixture = super::fixtures::store_path("isolated-package");
    let path = db.resolve_fixture_path(&path_fixture);

    // Closure size should be just this one path
    let result = conn.query_closure_size(&path);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), Size::from_bytes(1000));

    // Dependents should be just itself
    let result = conn.query_dependents(&path);
    assert!(result.is_ok());
    let dependents: Vec<_> = result.unwrap().collect();
    assert_eq!(dependents.len(), 1);

    conn.close().unwrap();
  }

  #[test]
  fn test_self_referential_path() {
    let db = edge_cases::create_self_reference_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = EagerDBConnection::new(&db_path_str);
    conn.connect().unwrap();

    let path_fixture = super::fixtures::store_path("self-referential");
    let path = db.resolve_fixture_path(&path_fixture);

    // Query should not hang or crash with self-reference
    let result = conn.query_closure_size(&path);
    assert!(result.is_ok());
    // Size should be counted once even with self-reference
    let size = result.unwrap();
    assert_eq!(size, Size::from_bytes(500));

    conn.close().unwrap();
  }

  #[test]
  fn test_circular_dependencies() {
    let db = edge_cases::create_circular_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = EagerDBConnection::new(&db_path_str);
    conn.connect().unwrap();

    // Test with different starting points in the cycle
    for letter in ["a", "b", "c"] {
      let path_fixture =
        super::fixtures::store_path(&format!("circular-{letter}"));
      let path = db.resolve_fixture_path(&path_fixture);

      // Query should not hang
      let result = conn.query_closure_size(&path);
      assert!(result.is_ok(), "Failed for circular-{letter}");

      // All three should have the same total (100 each = 300)
      let size = result.unwrap();
      assert_eq!(
        size,
        Size::from_bytes(300),
        "Wrong size for circular-{letter}"
      );
    }

    conn.close().unwrap();
  }

  #[test]
  fn test_disconnected_components() {
    let db = edge_cases::create_disconnected_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = EagerDBConnection::new(&db_path_str);
    conn.connect().unwrap();

    // Query first component
    let path1 =
      db.resolve_fixture_path(&super::fixtures::store_path("component1-a"));
    let result = conn.query_closure_size(&path1);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), Size::from_bytes(150));

    // Query second component
    let path2 =
      db.resolve_fixture_path(&super::fixtures::store_path("component2-a"));
    let result = conn.query_closure_size(&path2);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), Size::from_bytes(150));

    conn.close().unwrap();
  }

  #[test]
  fn test_wide_tree() {
    let db = edge_cases::create_wide_tree_test_db(100).unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = EagerDBConnection::new(&db_path_str);
    conn.connect().unwrap();

    let path =
      db.resolve_fixture_path(&super::fixtures::store_path("wide-root"));
    let result = conn.query_closure_size(&path);
    assert!(result.is_ok());

    // Root (1000) + 100 children (50 each) = 6000
    assert_eq!(result.unwrap(), Size::from_bytes(6000));

    // Should have 101 dependents (root + 100 children)
    let result = conn.query_dependents(&path);
    assert!(result.is_ok());
    let dependents: Vec<_> = result.unwrap().collect();
    assert_eq!(dependents.len(), 101);

    conn.close().unwrap();
  }

  #[test]
  fn test_deep_chain() {
    // Test with depth of 100
    let db = edge_cases::create_deep_chain_test_db(100).unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = LazyDBConnection::new(&db_path_str);
    conn.connect().unwrap();

    let path = db.resolve_fixture_path(&super::fixtures::store_path("deep-0"));
    let result = conn.query_closure_size(&path);
    assert!(result.is_ok());

    // 100 paths * 100 bytes each = 10000
    assert_eq!(result.unwrap(), Size::from_bytes(10000));

    // Should have 100 dependents
    let result = conn.query_dependents(&path);
    assert!(result.is_ok());
    let dependents: Vec<_> = result.unwrap().collect();
    assert_eq!(dependents.len(), 100);

    conn.close().unwrap();
  }

  #[test]
  fn test_special_characters_in_paths() {
    let db = edge_cases::create_special_chars_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    let mut conn = EagerDBConnection::new(&db_path_str);
    conn.connect().unwrap();

    // Query path with various special characters
    let path = db.resolve_fixture_path(&super::fixtures::store_path(
      "package-with-dashes-and-dots-1.2.3",
    ));
    let result = conn.query_closure_size(&path);
    assert!(result.is_ok());
    // 100 (parent) + 50 (child) = 150
    assert_eq!(result.unwrap(), Size::from_bytes(150));

    conn.close().unwrap();
  }

  #[test]
  fn test_both_backends_produce_same_results() {
    let db = edge_cases::create_circular_test_db().unwrap();
    let db_path_str = db.db_path().to_string_lossy().to_string();

    // Eager backend
    let mut eager = EagerDBConnection::new(&db_path_str);
    eager.connect().unwrap();

    // Lazy backend
    let mut lazy = LazyDBConnection::new(&db_path_str);
    lazy.connect().unwrap();

    let path =
      db.resolve_fixture_path(&super::fixtures::store_path("circular-a"));

    // Both should produce the same closure size
    let eager_size = eager.query_closure_size(&path).unwrap();
    let lazy_size = lazy.query_closure_size(&path).unwrap();
    assert_eq!(eager_size, lazy_size);

    // Both should produce the same number of dependents
    let eager_count = eager.query_dependents(&path).unwrap().count();
    let lazy_count = lazy.query_dependents(&path).unwrap().count();
    assert_eq!(eager_count, lazy_count);

    eager.close().unwrap();
    lazy.close().unwrap();
  }
}
