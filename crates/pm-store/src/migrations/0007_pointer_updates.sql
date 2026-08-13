-- High-water mark for signed mailbox-pointer updates from this contact
-- (docs/PRD.md §8) — the `updated_at` of the last one accepted, so a
-- stale or replayed update (e.g. from re-syncing a Server mailbox's
-- retained history after a restore) can be told apart from a genuinely
-- newer one. 0 for a freshly-paired contact: any real update's timestamp
-- is accepted as the first.
ALTER TABLE contacts ADD COLUMN pointer_updated_at INTEGER NOT NULL DEFAULT 0;
