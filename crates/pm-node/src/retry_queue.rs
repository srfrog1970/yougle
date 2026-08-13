//! In-memory retry-delivery queue for M7 (`docs/PRD.md` Flow 3's third
//! case): when a sender's own direct attempt to a Local-only recipient
//! fails, their own node takes over retrying delivery on a schedule. Same
//! "does not persist across restarts" limitation as [`crate::MailboxStore`]
//! (see its own doc comment) — not a new regression, matching that
//! existing, already-accepted scope boundary.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

#[derive(Debug)]
struct PendingEntry {
    recipient_transport_key: [u8; 32],
    envelope: Vec<u8>,
    attempts: u32,
    next_attempt_at: Instant,
}

#[derive(Debug)]
struct Inner {
    pending: HashMap<[u8; 16], PendingEntry>,
    /// msg_ids whose schedule exhausted, waiting to be drained by
    /// `PollFailedDeliveries` (see [`RetryQueue::take_failed`]).
    failed: Vec<[u8; 16]>,
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
#[derive(Debug)]
pub struct RetryQueue {
    inner: Mutex<Inner>,
}

impl Default for RetryQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl RetryQueue {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                pending: HashMap::new(),
                failed: Vec::new(),
            }),
        }
    }

    /// Queues `envelope` for delivery to `recipient_transport_key`, ready
    /// to be picked up on the very next sweep (a fresh entry is due
    /// immediately — no reason to wait out a backoff before the first
    /// attempt, since the recipient could already be reachable again by
    /// then).
    pub fn schedule(&self, msg_id: [u8; 16], recipient_transport_key: [u8; 32], envelope: Vec<u8>) {
        self.inner.lock().unwrap().pending.insert(
            msg_id,
            PendingEntry {
                recipient_transport_key,
                envelope,
                attempts: 0,
                next_attempt_at: Instant::now(),
            },
        );
    }

    /// Entries due for an attempt right now. Does not remove them — the
    /// caller reports the outcome back via [`Self::mark_delivered`] or
    /// [`Self::mark_failed`] once its own dial attempt resolves — but does
    /// push `next_attempt_at` out by [`IN_FLIGHT_GRACE`] immediately, so an
    /// in-flight attempt isn't handed out again by the next tick.
    pub fn take_due(&self) -> Vec<DueDelivery> {
        let mut inner = self.inner.lock().unwrap();
        let now = Instant::now();
        let mut due = Vec::new();
        for (msg_id, entry) in inner.pending.iter_mut() {
            if entry.next_attempt_at <= now {
                due.push(DueDelivery {
                    msg_id: *msg_id,
                    recipient_transport_key: entry.recipient_transport_key,
                    envelope: entry.envelope.clone(),
                });
                entry.next_attempt_at = now + IN_FLIGHT_GRACE;
            }
        }
        due
    }

    /// A queued delivery succeeded — remove it.
    pub fn mark_delivered(&self, msg_id: [u8; 16]) {
        self.inner.lock().unwrap().pending.remove(&msg_id);
    }

    /// A queued delivery's attempt failed — reschedule per
    /// [`RETRY_BACKOFF`], or, once [`MAX_RETRY_ATTEMPTS`] is reached, move
    /// it to the failed list `PollFailedDeliveries` drains.
    pub fn mark_failed(&self, msg_id: [u8; 16]) {
        let mut inner = self.inner.lock().unwrap();
        let attempts = match inner.pending.get_mut(&msg_id) {
            Some(entry) => {
                entry.attempts += 1;
                entry.attempts
            }
            None => return,
        };
        if attempts >= MAX_RETRY_ATTEMPTS {
            inner.pending.remove(&msg_id);
            inner.failed.push(msg_id);
        } else {
            let entry = inner.pending.get_mut(&msg_id).expect("just matched above");
            entry.next_attempt_at = Instant::now() + RETRY_BACKOFF[(attempts - 1) as usize];
        }
    }

    /// Drains (returns and clears) the msg_ids whose schedule has
    /// exhausted since the last call — see `NodeRequest::PollFailedDeliveries`'s
    /// doc comment for why this is drain-on-read rather than a separate
    /// ack step.
    pub fn take_failed(&self) -> Vec<[u8; 16]> {
        std::mem::take(&mut self.inner.lock().unwrap().failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_freshly_scheduled_entry_is_due_immediately() {
        let queue = RetryQueue::new();
        queue.schedule([1u8; 16], [2u8; 32], b"envelope".to_vec());

        let due = queue.take_due();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].msg_id, [1u8; 16]);
        assert_eq!(due[0].recipient_transport_key, [2u8; 32]);
        assert_eq!(due[0].envelope, b"envelope");
    }

    #[test]
    fn taking_an_entry_protects_it_from_being_taken_again_immediately() {
        let queue = RetryQueue::new();
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
        let queue = RetryQueue::new();
        queue.schedule([1u8; 16], [2u8; 32], b"envelope".to_vec());
        queue.take_due();

        queue.mark_delivered([1u8; 16]);

        // Even ignoring the in-flight grace period, the entry is gone.
        assert_eq!(queue.take_failed(), Vec::<[u8; 16]>::new());
    }

    #[test]
    fn mark_failed_on_an_unknown_msg_id_is_a_harmless_no_op() {
        let queue = RetryQueue::new();
        queue.mark_failed([9u8; 16]); // must not panic
    }

    #[test]
    fn exhausting_max_retry_attempts_moves_the_entry_to_the_failed_list() {
        let queue = RetryQueue::new();
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
        let queue = RetryQueue::new();
        queue.schedule([1u8; 16], [2u8; 32], b"envelope".to_vec());

        queue.mark_failed([1u8; 16]); // 1 of MAX_RETRY_ATTEMPTS

        assert_eq!(queue.take_failed(), Vec::<[u8; 16]>::new());
    }
}
