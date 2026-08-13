-- The one-time key received from a contact during pairing, held until this
-- device's *first* outbound message to them lazily establishes the Olm
-- session (not at pairing time — Olm has exactly one session per pair,
-- established by whoever sends first; eagerly creating an outbound session
-- on both sides at pairing time creates two independent, mismatched
-- sessions instead of one shared one). Cleared once consumed.
ALTER TABLE contacts ADD COLUMN pending_otk BLOB;
