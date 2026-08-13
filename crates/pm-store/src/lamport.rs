//! The classic Lamport logical clock, persisted as a single row so it
//! survives restarts. `sent_at` (wall-clock) breaks ties between messages
//! with the same Lamport value, per `ARCHIT_1.MD` §3 ("Lamport counter plus
//! sender timestamp for ordering").

use rusqlite::Connection;

use crate::error::Result;

/// Advances the local clock by one (an event happened here) and returns the
/// new value.
pub fn tick(conn: &Connection) -> Result<u64> {
    conn.execute(
        "UPDATE local_clock SET lamport = lamport + 1 WHERE id = 0",
        [],
    )?;
    current(conn)
}

/// Merges an observed remote value into the local clock — the standard
/// Lamport rule, `local = max(local, remote) + 1` — and returns the new
/// local value.
pub fn observe(conn: &Connection, remote: u64) -> Result<u64> {
    conn.execute(
        "UPDATE local_clock SET lamport = MAX(lamport, ?1) + 1 WHERE id = 0",
        [remote as i64],
    )?;
    current(conn)
}

pub fn current(conn: &Connection) -> Result<u64> {
    let value: i64 = conn.query_row("SELECT lamport FROM local_clock WHERE id = 0", [], |row| {
        row.get(0)
    })?;
    Ok(value as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrations::run(&conn).unwrap();
        conn
    }

    #[test]
    fn starts_at_zero() {
        assert_eq!(current(&conn()).unwrap(), 0);
    }

    #[test]
    fn tick_increments_by_one_each_call() {
        let conn = conn();
        assert_eq!(tick(&conn).unwrap(), 1);
        assert_eq!(tick(&conn).unwrap(), 2);
        assert_eq!(tick(&conn).unwrap(), 3);
    }

    #[test]
    fn observe_jumps_ahead_of_a_larger_remote_value() {
        let conn = conn();
        tick(&conn).unwrap(); // local = 1
        assert_eq!(observe(&conn, 10).unwrap(), 11);
    }

    #[test]
    fn observe_still_advances_when_remote_is_behind_local() {
        let conn = conn();
        for _ in 0..5 {
            tick(&conn).unwrap();
        }
        // local = 5; a remote value behind local still causes an advance,
        // since observing an event is itself an event.
        assert_eq!(observe(&conn, 1).unwrap(), 6);
    }
}
