# Threat Model

What this product protects against, what it doesn't claim to, and — the part a one-line stub can't give you — exactly who can see what, based on what's actually implemented (`docs/PROTOCOL.md`) rather than the aspirational design. Grounded in `docs/PRD.md` §6 (Constraints) and §8 (Technical Constraints), expanded against the real code.

## What this protects against

- **Commercial surveillance.** No accounts, no phone number or email tied to identity, no shared infrastructure collecting usage data across users.
- **Casual network-level observation of message content.** End-to-end encryption (vodozemac Olm) — content is unreadable by any mailbox, Local or Server, and unreadable in transit.
- **A curious shared operator learning your contact graph.** There is no shared, third-party-operated mailbox anywhere in this design. Every mailbox — Local (client-side) or Server (self-hosted) — is owned by the person using it. This was a real threat actor in the original, pre-decentralization design; it no longer applies.

## What this does not claim to protect against

- **A global passive adversary** — an actor capable of observing all network traffic across the internet.
- **Compromise of a user's own device, or their own self-hosted Server mailbox, by an external attacker.** Standard self-hosting exposure, accepted by whoever runs that infrastructure.
- **n0's relay/discovery infrastructure** (`*.relay.n0.iroh.link`, `dns.iroh.link`) — see below. This is a real, structurally necessary third party distinct from "no shared community mailbox," and the existing "no shared infrastructure" framing elsewhere in this project's docs refers specifically to *messaging-content* infrastructure, not the transport layer underneath it.
- **A recipient's own mailbox operator learning approximate message frequency and timing**, whether that's a self-hosted node the recipient trusts themselves, or — see below — anyone else positioned to watch the connections to it.

## Who can see what

The honest core of a threat model is enumerating actors and what's actually visible to each, not just asserting "it's encrypted." Three real actors exist in the current design, beyond the two communicating parties themselves.

### A Server mailbox operator

Every `pm-node` is single-tenant, run by its own owner — but that owner (or anyone who compromises their node) can see:

- **Never**: message content, sender identity in the application protocol. `Write` requests carry only a one-time hash-preimage (`auth`), by design — no `mailbox_key` or identity check gates it, since the caller is a sender, not the owner.
- **Always**: the padded ciphertext blob itself, and — since acknowledged messages are retained, not deleted (`docs/PRD.md` §5/§8) — every message this mailbox has ever received, indefinitely, until a retention policy exists (open item, see below).
- **The size bucket** each message falls into (256/1024/4096 bytes) — not exact content length, but a coarse size class.
- **Count and arrival order** of messages (`blobs.id` is a simple autoincrement).
- **When the owner is online**, from the timing of their own `Fetch`/`Ack` polling.
- **A stable sender pseudonym across writes**, even without knowing the sender's real identity: iroh is dial-by-public-key, and a device reuses the same transport endpoint identity for every outbound call it ever makes (deterministically derived from its seed). A node operator watching connections over time can correlate "this same pseudonymous party has written N times," independent of message content.

None of this requires malice — it's inherent to running a mailbox at all. It's the same category of exposure any self-hosted service operator has over their own service, just scoped down to a single tenant instead of a community.

### n0's relay and discovery infrastructure

This is the gap most likely to be missed by a reading of "no shared infrastructure" that stops at the mailbox layer. Every device's iroh endpoint — for Local delivery, for talking to a Server mailbox, for a self-hosted node's own outbound retry dialing — is configured against n0's production infrastructure with no override:

- **Publishes its own address** to n0's DNS server on startup and on change.
- **Queries n0's DNS server to resolve any peer it wants to reach** — meaning n0 sees who is looking up whom, a direct timing/contact-graph signal to a third party, independent of message content.
- **Routes traffic through an n0-operated relay server** whenever a direct connection can't be established (confirmed in practice: the local NAT-simulation test in `deploy/nat-sim/` delivered messages over the real relay, since this test host's own NAT chain doesn't support hairpin loopback for a direct path).

None of this exposes message content — the relay carries only ciphertext, and DNS/pkarr records don't carry plaintext either. But it does mean n0 is a real, unavoidable shared party positioned to observe endpoint identities and connection timing for essentially every conversation in this system, today. This is architecturally similar in kind (though far smaller in scope of what it can see) to the "curious shared operator" concern the mailbox layer was explicitly redesigned to eliminate — it just wasn't eliminated at the transport layer, and isn't yet acknowledged as a distinct actor anywhere else in this project's docs.

### A network observer

Standard TLS 1.3/QUIC exposure: connection metadata (who's talking to whom, when, for how long, roughly how much data) is visible to anyone positioned to observe the traffic, exactly as with any other QUIC-based service. Message content is not. `pm-proto`'s padding (see `docs/PROTOCOL.md` §2) only bounds what an `Envelope`'s ciphertext length reveals — it does not extend to the wire messages carrying it: a `Fetch` response's length scales directly with how many messages are pending, and an `Ack` request's length scales with how many message IDs are being acknowledged. Neither is padded. Connection count, duration, and timing between two endpoints are observable to anyone who can see the QUIC handshake, independent of any application-level padding.

## Known gaps and residual risks

Concrete, current-code facts, not hypothetical — each of these is either already an open item elsewhere in this project or newly surfaced while writing this document:

- **`RegisterSlot`/`Write` are now rate-limited; nothing else is, deliberately.** A fixed-window counter (default 60 requests/minute, a placeholder not a tuned production number) keyed by the connecting peer's `EndpointId` — cryptographically authenticated by the QUIC/TLS handshake itself, not self-reported — throttles exactly the two request types with no owner-auth check (`Write` by design; `RegisterSlot` in case a `mailbox_key` ever leaks). `Fetch`/`Ack`/`PollFailedDeliveries` are exempt on purpose: they're already gated by `mailbox_key`, and a real app polls its own Server mailbox continuously while foregrounded, so throttling them risked breaking normal usage for no real security gain. This only bounds abuse *per identity* — it does nothing against a flood distributed across many distinct identities (e.g. many freshly-generated seeds each producing their own valid endpoint id), which remains unaddressed.
- **A captured `auth` value could theoretically be raced.** Write-authorization is one-time-use by design (a slot hash can only be consumed once), but "first successful presentation wins" isn't itself defended against a race between the legitimate sender and someone who obtained a copy of that value — a gap the design leans on the underlying TLS 1.3 connection encryption to close, not a gap the application protocol itself defends against.
- **Resolved: mailbox-pointer-update rejection could no longer distinguish a forged update from an honest clock-skew collision.** Replay/rollback protection used to be a strict `updated_at >` wall-clock check, so a legitimate update whose timestamp happened to be ≤ the last accepted one (device clock drift, most plausibly) was silently dropped, identically to how an actually-forged update was dropped. Now gated by a locally-persisted, monotonically-incrementing counter (`seq`, see `docs/PROTOCOL.md` §7) instead — immune to clock issues by construction, since it never involves comparing two different devices' clocks. Made safe across a restore (the one way a naive counter could regress and get a device's *future* updates permanently rejected) by including the counter's current value in that device's own encrypted backup bundle. Fixing this also surfaced and closed a separate, pre-existing gap: a contact's `transport_key` was never included in backups at all, so restoring a device previously broke Local-delivery to every existing Local-only contact — a real bug the new restore-safety test exercises alongside the seq one.
- **Backup files leak an approximate size signal the encrypted envelopes deliberately don't.** `Envelope`s are padded into fixed buckets specifically so ciphertext length doesn't reveal message length; the backup bundle (full message history, contacts, session state) has no equivalent padding — its encrypted size scales near-linearly with plaintext size, so a backup file's size is a rough proxy for how much history it contains. The PRD's own Open Items already flag that backup file format/storage guidance isn't yet specified; this is the concrete mechanism behind that gap.
- **Storage growth on Server mailboxes is unbounded** — acknowledged messages are retained, not deleted, to support planned future functionality, with no retention/quota policy defined yet (`docs/PRD.md`, Open Items).
- **A wrong SQLCipher key against an already-migrated database can silently succeed at open time** and only surface as a failure on first real read/write, for both `pm-store` and `pm-node`'s own storage. Documented in-code as an accepted, low-severity gap for a single-tenant system a supervisor restarts on crash — noted here since it's a real (if narrow) integrity-check-timing gap, not purely a UX nit.

## Multi-device and identity

`docs/PRD.md` scopes "one identity per device" out for the MVP as a product limitation, but it's worth stating the security shape underneath that: Olm session state (the actual double-ratchet keys used for message encryption) is generated fresh and randomly per device, independent of the deterministic seed-derived identity keys — it is **not** re-derived from the recovery phrase. Opening the same seed phrase on a second device without going through the explicit restore/backup-import flow would produce a *different* random Olm identity on that second device, even though both devices would share the same deterministic signing/mailbox/backup/transport keys. Restoring onto a new device (the only supported multi-device-adjacent path today) works by replacing the old device's Olm state with a recovered pickle, not by running two devices simultaneously under one shared ratchet. This is incidental to how vodozemac session keys are generated, not a deliberately engineered blast-radius property — worth knowing, not worth over-claiming as a designed-in security feature.

## Explicitly changed from the original design

There is no shared, third-party-operated community mailbox in this product. Every mailbox — Local (client-side) or Server (self-hosted) — is owned by its own user. This removes "a curious node operator" as a threat actor in its original, community-infrastructure form; the residual, much narrower version of that concern that remains is a self-hosted node's *own* owner (see above), which is qualitatively different (opt-in, single-tenant, no shared blast radius across strangers).

## Open questions

- **Whether the original slot-hash/tag-chain anti-enumeration scheme is still needed.** Today's `RegisterSlot`/`Write` mechanism provides exactly one property: proof that a sender completed pairing and can produce a valid write-authorization value (`pm_proto::derive(pair_secret, "auth", n, 32)`) — it does not hide contact-graph size from anyone, because there's no longer a third-party operator whose view it would need to be hidden from (a mailbox's own owner already has direct database access to their own contact/slot count). No architectural decision record exists yet resolving whether any enumeration-hiding property is still worth building for a different reason (e.g., hiding contact count from someone who compromises a node without being its owner) — carried forward from `docs/PRD.md`'s Open Items, unresolved here.
- **Whether n0's relay/discovery role warrants its own explicit mitigation**, given it's a real third party with visibility into endpoint identities and connection timing across effectively every conversation in the system today. Disclosure is now done — the top-level `README.md` and `docs/PRD.md` §6 both state this plainly — but no mitigation (e.g. a self-hostable relay/discovery option) exists or is planned yet.
