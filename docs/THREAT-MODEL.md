# Threat Model

Summarized from `docs/PRD.md` (Section 6, Constraints). To be expanded as the protocol spec is written — this is a starting statement, not a complete analysis.

## What this protects against

- Commercial surveillance: no shared infrastructure, no accounts, no phone/email tied to identity.
- Casual network-level observation of message content: end-to-end encryption (vodozemac), content and framing unreadable by any mailbox.

## What this does not claim to protect against

- A global passive adversary (e.g., an actor capable of observing all network traffic).
- Compromise of a user's own device or their own self-hosted Server mailbox by an external attacker — this is standard self-hosting exposure, accepted by the person who owns that infrastructure.

## Explicitly changed from the original design

There is no shared, third-party-operated community mailbox in this product. Every mailbox — Local (client-side) or Server (self-hosted) — is owned by its own user. This removes "a curious node operator" as a threat actor entirely; it was only ever relevant when infrastructure was shared.

## Open questions

- Whether the original slot-hash/tag-chain anti-enumeration scheme (designed to hide contact counts from a shared operator) is still needed now that no shared operator exists. See `docs/PRD.md`, Open Items.
