CREATE TABLE registered_slots (
    slot_hash BLOB PRIMARY KEY
);

CREATE TABLE blobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    blob BLOB NOT NULL,
    delivered INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE backup (
    id INTEGER PRIMARY KEY CHECK (id = 0),
    blob BLOB NOT NULL
);

CREATE TABLE retry_entries (
    msg_id BLOB PRIMARY KEY,
    recipient_transport_key BLOB NOT NULL,
    envelope BLOB NOT NULL,
    attempts INTEGER NOT NULL,
    next_attempt_at_ms INTEGER NOT NULL
);

CREATE TABLE failed_deliveries (
    msg_id BLOB PRIMARY KEY
);
