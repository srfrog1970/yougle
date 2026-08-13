-- Per-contact pairing state: the shared secret from mutual QR pairing
-- (stand-in for now, since pairing itself isn't built yet — see pm-core),
-- and this device's own write counter for deriving fresh write-auth values
-- per outgoing message to that contact. Both sides derive the same
-- sequence from the same shared secret, so the recipient can pre-register
-- matching slot hashes ahead of time.
ALTER TABLE contacts ADD COLUMN pair_secret BLOB;
ALTER TABLE contacts ADD COLUMN next_write_n INTEGER NOT NULL DEFAULT 0;

-- High-water mark of node-assigned blob ids already processed by sync(),
-- so re-fetching (the node retains everything, never deletes) doesn't
-- reprocess old messages.
ALTER TABLE account ADD COLUMN last_synced_blob_id INTEGER NOT NULL DEFAULT 0;
