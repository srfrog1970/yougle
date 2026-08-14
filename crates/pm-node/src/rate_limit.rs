//! Per-identity request throttling for the node protocol. `Write` in
//! particular has no owner-auth check at all by design (the caller is a
//! sender, not the owner), gated only by a one-time write-authorization
//! proof — see `handler.rs`'s own doc comment — so it's the request type
//! most exposed to being hammered. A plain fixed-window counter, keyed by
//! the connecting peer's `EndpointId`: cryptographically authenticated by
//! the QUIC/TLS handshake itself (see `iroh::endpoint::Connection::
//! remote_id`'s own docs), so unlike anything self-reported in a request
//! body, it isn't something a peer can lie about.
//!
//! Applied only to `RegisterSlot`/`Write` (see `handler.rs`'s `accept`),
//! not every request type — deliberately excludes routine
//! owner-authenticated polling (`Fetch`/`Ack`/`PollFailedDeliveries`),
//! which is already gated by `mailbox_key` and can be legitimately
//! frequent (a real app polls its own Server mailbox continuously while
//! foregrounded, per `docs/PRD.md` §5). Throttling that too would risk
//! breaking normal usage without meaningfully raising the bar for an
//! attacker who already has a valid `mailbox_key` — confirmed the hard
//! way: an earlier, uniform-across-every-request-type version of this
//! limiter broke `pm-core`'s own M7 retry-exhaustion test, whose polling
//! loop legitimately exceeds any threshold sized for `RegisterSlot`/
//! `Write` abuse specifically.
//!
//! Hand-rolled rather than a `governor`/token-bucket dependency, matching
//! `retry_queue`'s own wall-clock-ms scheme — this crate's established
//! preference for small, self-contained mechanisms over a new dependency
//! for something this simple.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iroh::EndpointId;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after 1970")
        .as_millis() as i64
}

#[derive(Debug)]
struct Window {
    started_at_ms: i64,
    count: u32,
}

/// A fixed-window request counter per remote identity. `max_per_window`
/// and `window` are constructor parameters rather than fixed constants
/// specifically so tests can use tiny values instead of real time-based
/// sleeps; `spawn()` in `lib.rs` is the one place that picks the real
/// (placeholder — see its own doc comment) production values.
#[derive(Debug)]
pub struct RateLimiter {
    max_per_window: u32,
    window: Duration,
    windows: Mutex<HashMap<EndpointId, Window>>,
}

impl RateLimiter {
    pub fn new(max_per_window: u32, window: Duration) -> Self {
        Self {
            max_per_window,
            window,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// `true` if `remote_id` is still within its budget for the current
    /// window (and counts this call against it); `false` if it's already
    /// over the limit. A window that's already expired is reset rather
    /// than left to accumulate, so a remote id that goes quiet and comes
    /// back later starts fresh.
    pub fn allow(&self, remote_id: EndpointId) -> bool {
        let now = now_ms();
        let mut windows = self.windows.lock().unwrap();
        let entry = windows.entry(remote_id).or_insert(Window {
            started_at_ms: now,
            count: 0,
        });

        if now.saturating_sub(entry.started_at_ms) >= self.window.as_millis() as i64 {
            entry.started_at_ms = now;
            entry.count = 0;
        }

        if entry.count >= self.max_per_window {
            return false;
        }
        entry.count += 1;
        true
    }

    /// Sweeps out entries whose window has already fully expired — called
    /// once per `SWEEP_INTERVAL` tick alongside the M7 retry sweep (see
    /// `lib.rs`), not on its own separate task. Without this, tracking a
    /// distinct `HashMap` entry per remote id would itself be an unbounded
    /// memory-growth vector keyed by an attacker-controllable value —
    /// exactly the kind of thing this module exists to guard against.
    /// Bounds growth by "how many distinct endpoint ids connected in the
    /// last window," not all-time history.
    pub fn prune_expired(&self) {
        let now = now_ms();
        let window_ms = self.window.as_millis() as i64;
        self.windows
            .lock()
            .unwrap()
            .retain(|_, w| now.saturating_sub(w.started_at_ms) < window_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not every 32-byte value is a valid Ed25519 point, so a distinct
    /// `EndpointId` per test needs deriving from a `SecretKey` (scalar
    /// multiplication always yields a valid point) rather than parsing
    /// arbitrary bytes directly.
    fn endpoint_id(byte: u8) -> EndpointId {
        iroh::SecretKey::from_bytes(&[byte; 32]).public()
    }

    #[test]
    fn allows_up_to_the_cap_then_rejects() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        let id = endpoint_id(1);

        assert!(limiter.allow(id));
        assert!(limiter.allow(id));
        assert!(limiter.allow(id));
        assert!(
            !limiter.allow(id),
            "the 4th request within the window must be rejected"
        );
    }

    #[test]
    fn distinct_remote_ids_have_independent_budgets() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let a = endpoint_id(1);
        let b = endpoint_id(2);

        assert!(limiter.allow(a));
        assert!(!limiter.allow(a), "a is already over its own budget");
        assert!(limiter.allow(b), "b's budget is independent of a's");
    }

    #[test]
    fn resets_after_the_window_elapses() {
        let limiter = RateLimiter::new(1, Duration::from_millis(20));
        let id = endpoint_id(1);

        assert!(limiter.allow(id));
        assert!(!limiter.allow(id));

        std::thread::sleep(Duration::from_millis(30));
        assert!(
            limiter.allow(id),
            "a new window should grant a fresh budget"
        );
    }

    #[test]
    fn prune_expired_removes_stale_entries_but_keeps_active_ones() {
        let limiter = RateLimiter::new(5, Duration::from_millis(20));
        let stale = endpoint_id(1);
        let active = endpoint_id(2);

        assert!(limiter.allow(stale));
        std::thread::sleep(Duration::from_millis(30));
        assert!(limiter.allow(active)); // starts a fresh, still-live window

        limiter.prune_expired();

        assert_eq!(limiter.windows.lock().unwrap().len(), 1);
        assert!(limiter.windows.lock().unwrap().contains_key(&active));
    }
}
