//! Minimal linear migration runner. Applied SQL files are tracked via
//! `PRAGMA user_version`; add new migrations by appending to `MIGRATIONS`
//! and never editing an already-shipped one.

use rusqlite::Connection;

use crate::error::Result;

const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_init.sql"),
    include_str!("migrations/0002_account.sql"),
    include_str!("migrations/0003_pairing.sql"),
    include_str!("migrations/0004_pending_otk.sql"),
    include_str!("migrations/0005_own_server_addr.sql"),
    include_str!("migrations/0006_transport_and_status.sql"),
    include_str!("migrations/0007_pointer_updates.sql"),
    include_str!("migrations/0008_pointer_broadcast_seq.sql"),
];

pub fn run(conn: &Connection) -> Result<()> {
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
        run(&conn).unwrap(); // must not error re-applying / re-inserting local_clock's seed row

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
    }

    #[test]
    fn creates_expected_tables() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            names,
            vec!["account", "contacts", "local_clock", "messages", "sessions"]
        );
    }
}
