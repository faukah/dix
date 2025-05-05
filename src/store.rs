use async_std::task;
use sqlx::{Connection, SqliteConnection};
use std::future;

// executes a query on the nix db directly
// to gather all derivations that the derivation given by the path
// depends on
//
// in the future, we might wan't to switch to async
pub fn get_packages(path: &std::path::Path) -> Result<Vec<String>, String> {
    let p = path
        .canonicalize()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or("Could not convert resolve path")?;

    task::block_on(async {
        struct Col {
            path: String,
        }
        let mut conn = SqliteConnection::connect("sqlite:///nix/var/nix/db/db.sqlite")
            .await
            .map_err(|_| "Could not establish DB connection")?;
        let query_res = sqlx::query_as!(
            Col,
            "
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
            ",
            p
        )
        .fetch_all(&mut conn)
        .await
        .map_err(|_| "Could not execute package query")?
        .into_iter()
        .map(|c| c.path)
        .collect();
        Ok::<_, String>(query_res)
    })
}

// executes a query on the nix db directly
// to get the total closure size of the derivation
// by summing up the nar size of all derivations
// depending on the derivation
//
// in the future, we might wan't to switch to async
pub fn get_closure_size(path: &std::path::Path) -> Result<i64, String> {
    let p = path
        .canonicalize()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or("Could not convert resolve path")?;
    let size = task::block_on(async {
        struct Res {
            sum: Option<i64>,
        }
        let mut conn = SqliteConnection::connect("sqlite:///nix/var/nix/db/db.sqlite")
            .await
            .map_err(|_| "Could not establish DB connection")?;
        let query_res = sqlx::query_as!(
            Res,
            "
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
            ",
            p
        )
        .fetch_one(&mut conn)
        .await
        .map_err(|_| "Could not execute package query")?
        .sum
        .ok_or("Could not get closure size sum")?;
        Ok::<_, String>(query_res)
    });
    size
}

//
//
