-- A contact's iroh transport public key, shared at pairing time — lets
-- this device dial them directly for Local-to-local delivery (see M6).
-- Same opaque-bytes convention as server_addr.
ALTER TABLE contacts ADD COLUMN transport_key BLOB;

-- Delivery status for an outgoing message: 'sent' once written to the
-- recipient's Server (or delivered synchronously via direct P2P), then
-- 'delivered' once a receipt comes back, or 'failed' if delivery is known
-- to have failed. NULL for incoming messages — status isn't a concept
-- that applies to something already received.
ALTER TABLE messages ADD COLUMN status TEXT CHECK (status IN ('sent', 'delivered', 'failed'));
