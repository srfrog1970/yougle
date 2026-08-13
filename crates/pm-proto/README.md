# pm-proto

Envelopes, tag derivation, and wire-format versioning. Shared verbatim between `pm-core` (client) and `pm-node` (server mailbox binary) so framing and derivation logic can never drift between the two.

Status: not yet started (M0).

The original slot-hash/tag-chain padding scheme was designed to hide contact-graph information from a shared community mailbox operator. Since the PRD (v2.0) removed community mailboxes in favor of per-user Local/Server mailboxes, whether this scheme is still needed in its original form is an open question — see `docs/PRD.md`, Open Items.
