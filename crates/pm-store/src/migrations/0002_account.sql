-- The device's own vodozemac Account pickle (one-time keys, ratchet
-- identity keys), plus a record of where each contact can be reached, so
-- pm-core can pick a delivery target without needing DHT lookup.
CREATE TABLE account (
    id INTEGER PRIMARY KEY CHECK (id = 0),
    pickle BLOB NOT NULL,
    updated_at INTEGER NOT NULL
);

ALTER TABLE contacts ADD COLUMN server_addr BLOB;
