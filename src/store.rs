use crate::error::AppError;
use rusqlite::Connection;

// Use type alias for Result with our custom error type
type Result<T> = std::result::Result<T, AppError>;

const QUERY_PKGS: &str = "
WITH RECURSIVE
	graph(p) AS (
		SELECT id 
		FROM ValidPaths
		WHERE path = ?
	UNION
		SELECT reference FROM Refs
		JOIN graph ON referrer = p
	)
SELECT path from graph
JOIN ValidPaths ON id = p;
";

const QUERY_CLOSURE_SIZE: &str = "
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

/// executes a query on the nix db directly
/// to gather all derivations that the derivation given by the path
/// depends on
///
/// in the future, we might wan't to switch to async
pub fn get_packages(path: &std::path::Path) -> Result<Vec<String>> {
    let p: String = path.canonicalize()?.to_string_lossy().into_owned();
    let conn = Connection::open("/nix/var/nix/db/db.sqlite")?;

    let mut stmt = conn.prepare(QUERY_PKGS)?;
    let queried_pkgs: std::result::Result<Vec<String>, _> =
        stmt.query_map([p], |row| row.get(0))?.collect();
    Ok(queried_pkgs?)
}

/// executes a query on the nix db directly
/// to get the total closure size of the derivation
/// by summing up the nar size of all derivations
/// depending on the derivation
///
/// in the future, we might wan't to switch to async
pub fn get_closure_size(path: &std::path::Path) -> Result<i64> {
    let p: String = path.canonicalize()?.to_string_lossy().into_owned();
    let conn = Connection::open("/nix/var/nix/db/db.sqlite")?;

    let mut stmt = conn.prepare(QUERY_CLOSURE_SIZE)?;
    let queried_sum = stmt.query_row([p], |row| row.get(0))?;
    Ok(queried_sum)
}
