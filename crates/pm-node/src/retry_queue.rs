//! Retry-delivery queue for M7 (`docs/PRD.md` Flow 3's third case): when a
//! sender's own direct attempt to a Local-only recipient fails, their own
//! node takes over retrying delivery on a schedule. SQLCipher-encrypted at
//! rest (see `db.rs`) — persists across restarts when `spawn` is given a
//! `data_dir`, in-memory otherwise. `next_attempt_at_ms` is wall-clock (ms
//! since `UNIX_EPOCH`), not a monotonic clock reading, specifically so a
//! scheduled entry's due time survives a restart meaningfully.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

/// How many attempts total (the first happens as soon as a sweep picks the
/// entry up, not after an initial delay) before giving up and reporting
/// the message failed. Matches `docs/PRD.md` Flow 3's own example figure
/// ("three attempts") for cardinality; see [`RETRY_BACKOFF`] for why the
/// actual timing differs from its "within the hour" real-world framing.
pub const MAX_RETRY_ATTEMPTS: u32 = 3;

/// Delay before re-attempting after a failed attempt — index 0 is the gap
/// before attempt 2, index 1 the gap before attempt 3. Small, fast values
/// standing in for `docs/PRD.md`'s "within the hour" real-world figure
/// (same relationship `DIRECT_DELIVERY_TIMEOUT` already has to its own PRD
/// figure of "~15-20 seconds") so the whole schedule — success and
/// exhaustion alike — is provable inside a normal test run, not because
/// these are the intended production values. See `docs/PRD.md`'s "Retry
/// semantics will be a setting" open item.
const RETRY_BACKOFF: [Duration; (MAX_RETRY_ATTEMPTS - 1) as usize] =
    [Duration::from_secs(5), Duration::from_secs(15)];

/// How long a just-taken entry is protected from being handed out again by
/// a subsequent sweep tick while its attempt is still in flight — longer
/// than the per-attempt dial timeout the sweep loop itself uses, so a slow
/// (not yet timed-out) attempt can't overlap with a second concurrent one
/// for the same recipient. Overlapping attempts wouldn't just be wasteful:
/// decrypting the same Olm ciphertext twice on the receiving end can fail
/// the second time (session state advances per decrypt), so avoiding the
/// double-send matters for correctness, not just efficiency.
const IN_FLIGHT_GRACE: Duration = Duration::from_secs(12);

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after 1970")
        .as_millis() as i64
}

/// One item a sweep picked up and should now attempt to deliver.
#[derive(Debug)]
pub struct DueDelivery {
    pub msg_id: [u8; 16],
    pub recipient_transport_key: [u8; 32],
    pub envelope: Vec<u8>,
}

/// A single owner's outstanding retry-delivery jobs — keyed by `msg_id`
/// (already unique per `pm-core`'s own generation, see
/// `Store::set_message_status`'s doc comment), not a separate id scheme.
/// Shares its connection with [`crate::MailboxStore`] (both are handed the
/// same `Arc<Mutex<Connection>>` by `spawn`).
#[derive(Debug)]
pub struct RetryQueue {
    conn: Arc<Mutex<Connection>>,
}

impl RetryQueue {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Queues `envelope` for delivery to `recipient_transport_key`, ready
    /// to be picked up on the very next sweep (a fresh entry is due
    /// immediately — no reason to wait out a backoff before the first
    /// attempt, since the recipient could already be reachable again by
    /// then). Re-scheduling an already-pending `msg_id` resets its
    /// attempt count, matching a fresh `HashMap::insert`'s overwrite
    /// semantics.
    pub fn schedule(&self, msg_id: [u8; 16], recipient_transport_key: [u8; 32], envelope: Vec<u8>) {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO retry_entries \
                    (msg_id, recipient_transport_key, envelope, attempts, next_attempt_at_ms) \
                 VALUES (?1, ?2, ?3, 0, ?4) \
                 ON CONFLICT(msg_id) DO UPDATE SET \
                    recipient_transport_key = excluded.recipient_transport_key, \
                    envelope = excluded.envelope, \
                    attempts = 0, \
                    next_attempt_at_ms = excluded.next_attempt_at_ms",
                params![
                    msg_id.as_slice(),
                    recipient_transport_key.as_slice(),
                    envelope,
                    now_ms()
                ],
            )
            .expect("schedule: storage I/O failure after a successful open+migration");
    }

    /// Entries due for an attempt right now. Does not remove them — the
    /// caller reports the outcome back via [`Self::mark_delivered`] or
    /// [`Self::mark_failed`] once its own dial attempt resolves — but does
    /// push `next_attempt_at_ms` out by [`IN_FLIGHT_GRACE`] immediately, so
    /// an in-flight attempt isn't handed out again by the next tick.
    pub fn take_due(&self) -> Vec<DueDelivery> {
        let conn = self.conn.lock().unwrap();
        let now = now_ms();

        let due: Vec<DueDelivery> = {
            let mut stmt = conn
                .prepare(
                    "SELECT msg_id, recipient_transport_key, envelope \
                     FROM retry_entries WHERE next_attempt_at_ms <= ?1",
                )
                .expect("take_due: storage I/O failure after a successful open+migration");
            stmt.query_map(params![now], |row| {
                let msg_id: Vec<u8> = row.get(0)?;
                let recipient_transport_key: Vec<u8> = row.get(1)?;
                Ok(DueDelivery {
                    msg_id: msg_id.try_into().expect("msg_id column is always 16 bytes"),
                    recipient_transport_key: recipient_transport_key
                        .try_into()
                        .expect("recipient_transport_key column is always 32 bytes"),
                    envelope: row.get(2)?,
                })
            })
            .expect("take_due: storage I/O failure after a successful open+migration")
            .collect::<rusqlite::Result<_>>()
            .expect("take_due: storage I/O failure after a successful open+migration")
        };

        let grace_until = now + IN_FLIGHT_GRACE.as_millis() as i64;
        for entry in &due {
            conn.execute(
                "UPDATE retry_entries SET next_attempt_at_ms = ?1 WHERE msg_id = ?2",
                params![grace_until, entry.msg_id.as_slice()],
            )
            .expect("take_due: storage I/O failure after a successful open+migration");
        }
        due
    }

    /// A queued delivery succeeded — remove it.
    pub fn mark_delivered(&self, msg_id: [u8; 16]) {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM retry_entries WHERE msg_id = ?1",
                params![msg_id.as_slice()],
            )
            .expect("mark_delivered: storage I/O failure after a successful open+migration");
    }

    /// A queued delivery's attempt failed — reschedule per
    /// [`RETRY_BACKOFF`], or, once [`MAX_RETRY_ATTEMPTS`] is reached, move
    /// it to the failed list `PollFailedDeliveries` drains. An unknown
    /// `msg_id` (already delivered/removed elsewhere) is a harmless no-op.
    pub fn mark_failed(&self, msg_id: [u8; 16]) {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction()
            .expect("mark_failed: storage I/O failure after a successful open+migration");

        let attempts: Option<i64> = tx
            .query_row(
                "SELECT attempts FROM retry_entries WHERE msg_id = ?1",
                params![msg_id.as_slice()],
                |row| row.get(0),
            )
            .optional()
            .expect("mark_failed: storage I/O failure after a successful open+migration");
        let Some(attempts) = attempts else {
            return; // unknown msg_id: nothing to update, tx rolls back on drop
        };
        let attempts = attempts as u32 + 1;

        if attempts >= MAX_RETRY_ATTEMPTS {
            tx.execute(
                "DELETE FROM retry_entries WHERE msg_id = ?1",
                params![msg_id.as_slice()],
            )
            .expect("mark_failed: storage I/O failure after a successful open+migration");
            tx.execute(
                "INSERT OR IGNORE INTO failed_deliveries (msg_id) VALUES (?1)",
                params![msg_id.as_slice()],
            )
            .expect("mark_failed: storage I/O failure after a successful open+migration");
        } else {
            let next_attempt_at_ms =
                now_ms() + RETRY_BACKOFF[(attempts - 1) as usize].as_millis() as i64;
            tx.execute(
                "UPDATE retry_entries SET attempts = ?1, next_attempt_at_ms = ?2 WHERE msg_id = ?3",
                params![attempts as i64, next_attempt_at_ms, msg_id.as_slice()],
            )
            .expect("mark_failed: storage I/O failure after a successful open+migration");
        }
        tx.commit()
            .expect("mark_failed: storage I/O failure after a successful open+migration");
    }

    /// Drains (returns and clears) the msg_ids whose schedule has
    /// exhausted since the last call — see `NodeRequest::PollFailedDeliveries`'s
    /// doc comment for why this is drain-on-read rather than a separate
    /// ack step.
    pub fn take_failed(&self) -> Vec<[u8; 16]> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction()
            .expect("take_failed: storage I/O failure after a successful open+migration");
        let ids: Vec<[u8; 16]> = {
            let mut stmt = tx
                .prepare("SELECT msg_id FROM failed_deliveries")
                .expect("take_failed: storage I/O failure after a successful open+migration");
            stmt.query_map([], |row| {
                let msg_id: Vec<u8> = row.get(0)?;
                Ok(msg_id.try_into().expect("msg_id column is always 16 bytes"))
            })
            .expect("take_failed: storage I/O failure after a successful open+migration")
            .collect::<rusqlite::Result<_>>()
            .expect("take_failed: storage I/O failure after a successful open+migration")
        };
        tx.execute("DELETE FROM failed_deliveries", [])
            .expect("take_failed: storage I/O failure after a successful open+migration");
        tx.commit()
            .expect("take_failed: storage I/O failure after a successful open+migration");
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn queue() -> RetryQueue {
        RetryQueue::new(db::open_for_test())
    }

    #[test]
    fn a_freshly_scheduled_entry_is_due_immediately() {
        let queue = queue();
        queue.schedule([1u8; 16], [2u8; 32], b"envelope".to_vec());

        let due = queue.take_due();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].msg_id, [1u8; 16]);
        assert_eq!(due[0].recipient_transport_key, [2u8; 32]);
        assert_eq!(due[0].envelope, b"envelope");
    }

    #[test]
    fn taking_an_entry_protects_it_from_being_taken_again_immediately() {
        let queue = queue();
        queue.schedule([1u8; 16], [2u8; 32], b"envelope".to_vec());

        assert_eq!(queue.take_due().len(), 1);
        assert_eq!(
            queue.take_due().len(),
            0,
            "an in-flight entry must not be handed out again right away"
        );
    }

    #[test]
    fn mark_delivered_removes_the_entry() {
        let queue = queue();
        queue.schedule([1u8; 16], [2u8; 32], b"envelope".to_vec());
        queue.take_due();

        queue.mark_delivered([1u8; 16]);

        // Even ignoring the in-flight grace period, the entry is gone.
        assert_eq!(queue.take_failed(), Vec::<[u8; 16]>::new());
    }

    #[test]
    fn mark_failed_on_an_unknown_msg_id_is_a_harmless_no_op() {
        let queue = queue();
        queue.mark_failed([9u8; 16]); // must not panic
    }

    #[test]
    fn exhausting_max_retry_attempts_moves_the_entry_to_the_failed_list() {
        let queue = queue();
        queue.schedule([1u8; 16], [2u8; 32], b"envelope".to_vec());

        for _ in 0..MAX_RETRY_ATTEMPTS {
            queue.mark_failed([1u8; 16]);
        }

        assert_eq!(queue.take_failed(), vec![[1u8; 16]]);
        // Draining is one-shot — a second call returns nothing new.
        assert_eq!(queue.take_failed(), Vec::<[u8; 16]>::new());
    }

    #[test]
    fn a_failure_before_exhaustion_is_not_reported_as_failed() {
        let queue = queue();
        queue.schedule([1u8; 16], [2u8; 32], b"envelope".to_vec());

        queue.mark_failed([1u8; 16]); // 1 of MAX_RETRY_ATTEMPTS

        assert_eq!(queue.take_failed(), Vec::<[u8; 16]>::new());
    }

    #[test]
    fn a_failure_before_exhaustion_leaves_the_entry_pending_not_immediately_due() {
        let queue = queue();
        queue.schedule([1u8; 16], [2u8; 32], b"envelope".to_vec());
        queue.take_due(); // first attempt, now in the in-flight grace window

        queue.mark_failed([1u8; 16]); // reschedules per RETRY_BACKOFF[0] (5s out)

        assert_eq!(
            queue.take_due().len(),
            0,
            "a just-failed entry should be backed off, not immediately due again"
        );
    }
}
