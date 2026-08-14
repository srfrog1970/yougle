//! Minimal linear migration runner, mirroring `pm-store`'s own mechanism
//! (see `crates/pm-store/src/migrations.rs`) — a small, independent copy,
//! not a shared dependency (see `db.rs`'s module doc for why). Applied SQL
//! files are tracked via `PRAGMA user_version`; add new migrations by
//! appending to `MIGRATIONS` and never editing an already-shipped one.

use rusqlite::{Connection, Result};

const MIGRATIONS: &[&str] = &[include_str!("migrations/0001_init.sql")];

pub(crate) fn run(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let current = current as usize;

    for (i, migration) in MIGRATIONS.iter().enumerate().skip(current) {
        conn.execute_batch(migration)?;
        conn.pragma_update(None, "user_version", (i + 1) as i64)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_twice_is_a_noop_the_second_time() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        run(&conn).unwrap();

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
    }

    #[test]
    fn creates_expected_tables() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();

        // sqlite_sequence is SQLite's own internal bookkeeping table,
        // created automatically by blobs' AUTOINCREMENT column — not one
        // of ours.
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type = 'table' AND name != 'sqlite_sequence' ORDER BY name",
            )
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            names,
            vec![
                "backup",
                "blobs",
                "failed_deliveries",
                "registered_slots",
                "retry_entries",
            ]
        );
    }
}
