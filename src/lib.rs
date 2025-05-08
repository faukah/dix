use std::path::{
  Path,
  PathBuf,
};

use derive_more::Deref;
use ref_cast::RefCast;

pub mod error;
pub mod print;

pub mod store;

pub mod util;

#[derive(Deref, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DerivationId(i64);

#[derive(RefCast, Deref, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct StorePath(Path);

#[derive(Deref, Debug, Clone, PartialEq, Eq)]
pub struct StorePathBuf(PathBuf);
