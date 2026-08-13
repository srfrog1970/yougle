CREATE TABLE contacts (
    id INTEGER PRIMARY KEY,
    identity_key BLOB NOT NULL UNIQUE,
    curve25519_key BLOB NOT NULL,
    display_name TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE sessions (
    contact_id INTEGER PRIMARY KEY REFERENCES contacts (id) ON DELETE CASCADE,
    pickle BLOB NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    contact_id INTEGER NOT NULL REFERENCES contacts (id) ON DELETE CASCADE,
    msg_id BLOB NOT NULL,
    direction TEXT NOT NULL CHECK (direction IN ('outgoing', 'incoming')),
    lamport INTEGER NOT NULL,
    sent_at INTEGER NOT NULL,
    plaintext BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (contact_id, msg_id)
);

CREATE INDEX messages_by_contact_lamport ON messages (contact_id, lamport);

-- Single-row table holding this device's Lamport clock value.
CREATE TABLE local_clock (
    id INTEGER PRIMARY KEY CHECK (id = 0),
    lamport INTEGER NOT NULL
);

INSERT INTO local_clock (id, lamport) VALUES (0, 0);
