#![allow(clippy::mem_forget)]

mod db_eager;
mod db_lazy;
mod nix_command;
mod queries;

pub mod store {
  pub use crate::store::db_lazy::DBConnection;
}

use std::{
  fmt::Display,
  iter::Iterator,
  path::Path,
};

use anyhow::{
  Result,
  anyhow,
};
use log::warn;
use size::Size;

use crate::{
  DerivationId,
  StorePath,
  store::{
    nix_command::CommandBackend,
    store::DBConnection,
  },
};
/// The normal database connection
pub const DATABASE_PATH: &str = "file:/nix/var/nix/db/db.sqlite";
/// A backup database connection that can access the database
/// even in a read-only environment
///
/// This might produce incorrect results as the connection is not guaranteed
/// to be the only one accessing the database. (There might be e.g. a
/// `nixos-rebuild` modifying the database)
pub const DATABASE_PATH_IMMUTABLE: &str =
  "file:/nix/var/nix/db/db.sqlite?immutable=1";

/// Defines an interface for interacting with a Nix database.
///
/// This allows us to construct a backend that can fall back
/// to e.g. shell commands should something go wrong.
pub trait StoreBackend<'a> {
  fn connect(&mut self) -> Result<()>;
  fn connected(&self) -> bool;
  fn close(&mut self) -> Result<()>;
  fn query_closure_size(&self, path: &Path) -> Result<Size>;
  fn query_system_derivations(
    &self,
    system: &Path,
  ) -> Result<Box<dyn Iterator<Item = StorePath> + '_>>;
  fn query_dependents(
    &self,
    path: &Path,
  ) -> Result<Box<dyn Iterator<Item = StorePath> + '_>>;
  #[expect(dead_code)]
  fn query_dependency_graph(
    &self,
    path: &Path,
  ) -> Result<Box<dyn Iterator<Item = (DerivationId, DerivationId)> + '_>>;
}

/// wrapper trait for debug information
pub trait StoreBackendPrintable<'a>: StoreBackend<'a> + Display {}

impl<'a, T> StoreBackendPrintable<'a> for T where T: StoreBackend<'a> + Display {}

/// combines multiple store backends by falling back to the next one if the
/// current one fails.
///
/// Currently, the first backend that works when connecting is used.
pub struct CombinedStoreBackend<'a> {
  /// The underlying store backend implementations.
  backends: Vec<Box<dyn StoreBackendPrintable<'a>>>,
}

impl<'a> CombinedStoreBackend<'a> {
  pub fn new(backends: Vec<Box<dyn StoreBackendPrintable<'a>>>) -> Self {
    Self { backends }
  }
  // tries to execute a query until it succeeds or all connected backends have
  // been tried
  fn fallback_query<'b, F, Ret>(&'b self, query: F, path: &Path) -> Result<Ret>
  where
    F: Fn(&'b Box<dyn StoreBackendPrintable<'a>>, &Path) -> Result<Ret>,
  {
    let mut combined_err: Option<anyhow::Error> = None;
    // attempt to cycle through backends until a successful query is made
    for (i, backend) in self.backends.iter().enumerate() {
      if !backend.connected() {
        warn!(
          "Skipping backend {i} ({backend}) in query {path:?}: not connected"
        );
        continue;
      }
      let res = query(backend, path);
      match res {
        Ok(_) => return res,
        Err(err) => {
          warn!(
            "Failed to query path {path:?} on current backend {backend} \
             ({i}): {}",
            &err
          );
          combined_err = match combined_err {
            Some(combined) => Some(combined.context(err)),
            None => Some(err),
          };
        },
      }
    }
    warn!("All store backends for path {path:?} failed");
    Err(combined_err.unwrap_or_else(|| anyhow!("No internal stores to query.")))
  }
}

impl<'a> Default for CombinedStoreBackend<'a> {
  fn default() -> Self {
    CombinedStoreBackend::new(vec![
      Box::new(DBConnection::new(DATABASE_PATH)),
      Box::new(DBConnection::new(DATABASE_PATH_IMMUTABLE)),
      Box::new(CommandBackend),
    ])
  }
}

impl<'a> StoreBackend<'a> for CombinedStoreBackend<'a> {
  /// connects to all backends. Returns an error if all backends fail
  fn connect(&mut self) -> Result<()> {
    let mut combined_err: Option<anyhow::Error> = None;
    // connect, collecting the errors as we go
    for (i, backend) in self.backends.iter_mut().enumerate() {
      if let Err(err) = backend.connect() {
        warn!(
          "Unable to connect to store backend {i}: {backend}, trying next. \
           (error: {err})"
        );
        combined_err = match combined_err {
          Some(combined) => Some(combined.context(err)),
          None => Some(err),
        }
      }
    }
    let any_succeeded = self.backends.iter().any(|f| f.connected());
    // warn about encountered errors, even though there are fallbacks
    if let Some(err) = &combined_err
      && any_succeeded
    {
      warn!("Some backends failed to connect: {err}")
    }
    if any_succeeded {
      Ok(())
    } else {
      combined_err =
        combined_err.map(|err| err.context("All backends failed to connect."));
      Err(combined_err.unwrap_or_else(|| anyhow!("No backends to connect to.")))
    }
  }

  /// True if any backend is connected.
  fn connected(&self) -> bool {
    self.backends.iter().any(|backend| backend.connected())
  }

  /// Closes all connected backends.
  ///
  /// If some fail to close, the combined error is returned.
  fn close(&mut self) -> Result<()> {
    let mut combined_err: Option<anyhow::Error> = None;
    for (i, backend) in self.backends.iter_mut().enumerate() {
      if backend.connected() {
        if let Err(err) = backend.close() {
          warn!("Unable to close store backend {i}: {backend}. (error: {err})");
          combined_err = match combined_err {
            Some(combined) => Some(combined.context(err)),
            None => Some(err),
          };
        }
      }
    }
    if let Some(err) = combined_err {
      Err(err.context("One or more backends failed to close."))
    } else {
      Ok(())
    }
  }

  fn query_closure_size(&self, path: &Path) -> Result<Size> {
    self.fallback_query(
      |backend, path| (**backend).query_closure_size(path),
      path,
    )
  }

  fn query_system_derivations(
    &self,
    system: &Path,
  ) -> Result<Box<dyn Iterator<Item = StorePath> + '_>> {
    self.fallback_query(
      |backend, system| (**backend).query_system_derivations(system),
      system,
    )
  }

  fn query_dependents(
    &self,
    path: &Path,
  ) -> Result<Box<dyn Iterator<Item = StorePath> + '_>> {
    self
      .fallback_query(|backend, path| (**backend).query_dependents(path), path)
  }

  fn query_dependency_graph(
    &self,
    path: &Path,
  ) -> Result<Box<dyn Iterator<Item = (DerivationId, DerivationId)> + '_>> {
    self.fallback_query(
      |backend, path| (**backend).query_dependency_graph(path),
      path,
    )
  }
}

#[cfg(test)]
mod test {
  use std::{
    cell::RefCell,
    fmt,
  };

  use super::*;

  struct MockStoreBackend {
    name:         String,
    connected:    bool,
    fail_connect: bool,
    fail_query:   bool,
    query_called: RefCell<bool>,
  }

  impl MockStoreBackend {
    fn new(name: &str, fail_connect: bool, fail_query: bool) -> Self {
      Self {
        name: name.to_string(),
        connected: false,
        fail_connect,
        fail_query,
        query_called: RefCell::new(false),
      }
    }
  }

  impl Display for MockStoreBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      write!(f, "MockStoreBackend({})", self.name)
    }
  }

  impl<'a> StoreBackend<'a> for MockStoreBackend {
    fn connect(&mut self) -> Result<()> {
      if self.fail_connect {
        Err(anyhow!("Connection failed"))
      } else {
        self.connected = true;
        Ok(())
      }
    }

    fn connected(&self) -> bool {
      self.connected
    }

    fn close(&mut self) -> Result<()> {
      self.connected = false;
      Ok(())
    }

    fn query_closure_size(&self, _path: &Path) -> Result<Size> {
      *self.query_called.borrow_mut() = true;
      if self.fail_query {
        Err(anyhow!("Query failed"))
      } else {
        Ok(Size::from_bytes(100))
      }
    }

    fn query_system_derivations(
      &self,
      _system: &Path,
    ) -> Result<Box<dyn Iterator<Item = StorePath> + '_>> {
      unimplemented!()
    }

    fn query_dependents(
      &self,
      _path: &Path,
    ) -> Result<Box<dyn Iterator<Item = StorePath> + '_>> {
      unimplemented!()
    }

    fn query_dependency_graph(
      &self,
      _path: &Path,
    ) -> Result<Box<dyn Iterator<Item = (DerivationId, DerivationId)> + '_>>
    {
      unimplemented!()
    }
  }

  #[test]
  fn test_connect_fallback() {
    let f1 = Box::new(MockStoreBackend::new("f1", true, false));
    let f2 = Box::new(MockStoreBackend::new("f2", false, false));
    let mut combined = CombinedStoreBackend::new(vec![f1, f2]);

    assert!(combined.connect().is_ok());
    assert!(combined.connected());
  }

  #[test]
  fn test_connect_all_fail() {
    let f1 = Box::new(MockStoreBackend::new("f1", true, false));
    let f2 = Box::new(MockStoreBackend::new("f2", true, false));
    let mut combined = CombinedStoreBackend::new(vec![f1, f2]);

    assert!(combined.connect().is_err());
    assert!(!combined.connected());
  }

  #[test]
  fn test_query_fallback() {
    let f1 = Box::new(MockStoreBackend::new("f1", false, true));
    let f2 = Box::new(MockStoreBackend::new("f2", false, false));
    let mut combined = CombinedStoreBackend::new(vec![f1, f2]);

    combined.connect().unwrap();

    let res = combined.query_closure_size(Path::new("/dummy"));
    assert!(res.is_ok());
    assert_eq!(res.unwrap(), Size::from_bytes(100));
  }

  #[test]
  fn test_query_skip_unconnected() {
    let f1 = Box::new(MockStoreBackend::new("f1", true, false));
    let f2 = Box::new(MockStoreBackend::new("f2", false, false));
    let mut combined = CombinedStoreBackend::new(vec![f1, f2]);

    combined.connect().unwrap(); // f1 fails, f2 succeeds

    let res = combined.query_closure_size(Path::new("/dummy"));
    assert!(res.is_ok());
    assert_eq!(res.unwrap(), Size::from_bytes(100));

    let f1 = Box::new(MockStoreBackend::new("f1", true, false));
    let f2 = Box::new(MockStoreBackend::new("f2", true, false));
    let f3 = Box::new(MockStoreBackend::new("f3", false, false));
    let mut combined = CombinedStoreBackend::new(vec![f1, f2, f3]);
    combined.connect().unwrap();

    let res = combined.query_closure_size(Path::new("/dummy"));
    assert_eq!(res.unwrap(), Size::from_bytes(100));
    assert!(combined.connect().is_ok());
    assert!(combined.connected());
  }

  #[test]
  fn test_query_all_fail() {
    let f1 = Box::new(MockStoreBackend::new("f1", false, true));
    let f2 = Box::new(MockStoreBackend::new("f2", false, true));
    let mut combined = CombinedStoreBackend::new(vec![f1, f2]);

    combined.connect().unwrap();

    let res = combined.query_closure_size(Path::new("/dummy"));
    assert!(res.is_err());
  }
}
