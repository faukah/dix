use std::collections::HashMap;

use crate::error::AppError;
use rusqlite::Connection;

// Use type alias for Result with our custom error type
type Result<T> = std::result::Result<T, AppError>;

const DATABASE_URL: &str = "/nix/var/nix/db/db.sqlite";

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
SELECT id, path from graph
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

const QUERY_DEPENDENCY_GRAPH: &str = "
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

/// executes a query on the nix db directly
/// to gather all derivations that the derivation given by the path
/// depends on
///
/// The ids of the derivations in the database are returned as well, since these
/// can be used to later convert nodes (represented by the the ids) of the
/// dependency graph to actual paths
///
/// in the future, we might wan't to switch to async
pub fn get_packages(path: &std::path::Path) -> Result<Vec<(i64, String)>> {
    let p: String = path.canonicalize()?.to_string_lossy().into_owned();
    let conn = Connection::open(DATABASE_URL)?;

    let mut stmt = conn.prepare(QUERY_PKGS)?;
    let queried_pkgs: std::result::Result<Vec<(i64, String)>, _> = stmt
        .query_map([p], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect();
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
    let conn = Connection::open(DATABASE_URL)?;

    let mut stmt = conn.prepare(QUERY_CLOSURE_SIZE)?;
    let queried_sum = stmt.query_row([p], |row| row.get(0))?;
    Ok(queried_sum)
}

/// returns the complete dependency graph of
/// of the derivation as an adjacency list. The nodes are
/// represented by the DB ids
///
/// We might want to collect the paths in the graph directly as
/// well in the future, depending on how much we use them
/// in the operations on the graph
///
/// The mapping from id to graph can be obtained by using [``get_packages``]
pub fn get_dependency_graph(path: &std::path::Path) -> Result<HashMap<i64, Vec<i64>>> {
    let p: String = path.canonicalize()?.to_string_lossy().into_owned();
    let conn = Connection::open(DATABASE_URL)?;

    let mut stmt = conn.prepare(QUERY_DEPENDENCY_GRAPH)?;
    let mut adj = HashMap::<i64, Vec<i64>>::new();
    let queried_sum = stmt.query_map([p], |row| Ok::<(i64, i64), _>((row.get(0)?, row.get(1)?)))?;
    for row in queried_sum {
        let (from, to) = row?;
        adj.entry(from).or_default().push(to);
    }

    Ok(adj)
}
