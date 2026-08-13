-- This device's own Server mailbox address (if any), so the app doesn't
-- have to be handed it fresh by the caller every session (see pm-core's
-- M5 Client changes). Same opaque-bytes convention as contacts.server_addr.
ALTER TABLE account ADD COLUMN own_server_addr BLOB;
