# Private Messenger — Product Requirements Document

*Draft v2.0 — 2026-08-12*

---

## Revision notes (v1.0 → v2.0)

The mailbox model changed substantially during review of the user flows. Recording what changed and why, so the reasoning survives.

| v1.0 said | Now | Why |
|---|---|---|
| New users join a shared community mailbox via an invite code | No community mailbox exists. Every user gets an automatic, client-side **Local** mailbox on install; any user may optionally add a single self-hosted **Server** mailbox | Removes the shared third-party trust boundary and the invite-ticket bootstrap problem entirely — the product is fully decentralized, every mailbox is owned by its own user |
| Delivery is pull-only; the recipient's mailbox holds messages until fetched | Delivery depends on setup: instant live delivery between two Local-only users, a single durable write if the recipient has a Server mailbox, or the sender's own Server mailbox retrying on a schedule if the recipient doesn't | Supports offline/async delivery without requiring anyone to trust shared infrastructure |
| Message history is never recoverable | Message history and contacts are recoverable automatically via a Server mailbox, or manually via a user-initiated encrypted export/import | Needed so the user can build functionality on top of retained message data (phase 2) |
| Messages are deleted from the mailbox once acknowledged | Messages are retained on a Server mailbox after acknowledgment | Supports the same phase-2 goal; introduces an unresolved storage-growth question (see Open Items) |
| Recovery phrase must be recorded/verified at onboarding | Recovery phrase is generated silently; viewable on demand in Settings, with a single one-time non-blocking nudge | Reduces onboarding friction for a non-technical audience |

---

## 1. Primary Users

The primary users are general consumers who are uneasy about how mainstream messaging platforms handle their data, but who do not face a specific, active threat (such as targeted surveillance or persecution). They are not technical: they have no prior experience with cryptographic concepts such as seed phrases, key pairs, or key management, and the product must not require them to acquire this knowledge to use it safely.

Because the architecture introduces friction not found in mainstream messengers — in-person pairing and, by default, live-only delivery instead of push notifications — the product must justify this friction in terms a non-technical user already understands (e.g., "no company can read your messages") rather than in technical terms.

Running a self-hosted Server mailbox (e.g., on a Raspberry Pi) is an optional, advanced capability. It is not expected of, or required from, the primary non-technical persona described above — they can use the full product in Local-only mode. It's aimed at a smaller subset of more technical users, who may include some of the primary users' own contacts.

## 2. Problems to Solve

This product addresses five problems with mainstream messaging platforms:

1. **Readable message content.** Even where messages are described as encrypted, the platform operator often retains the technical ability to access content — through key escrow, server-side processing, or account recovery mechanisms. This product uses end-to-end encryption where no operator has such a mechanism.
2. **Metadata collection.** Independent of message content, platforms typically observe and retain who a user communicates with, how often, and when. This product has no shared infrastructure at all by default — a user's Local mailbox is their own device, and an optional Server mailbox is infrastructure they alone own and control.
3. **Centralized, real-world-linked identity.** Mainstream platforms tie a messaging identity to a phone number or email address, both of which are identifying and typically already linked to other services. This product uses a self-generated cryptographic identity with no phone number, email, or central account required.
4. **Platform lock-in.** Leaving a mainstream platform typically means losing your contact list and message history. This product always allows identity recovery from a seed phrase, and allows contacts and message history to be recovered either automatically (if the user runs their own Server mailbox) or via a manually managed encrypted backup file — independent of any company, and without depending on infrastructure the user doesn't control.
5. **Push-notification infrastructure as a surveillance channel.** Apple's and Google's push services see device tokens and message timing even when content is encrypted, creating a centralized record outside the app's own control. This product uses no push notifications anywhere in its delivery model.

## 3. Capabilities

The MVP product provides the following capabilities:

- **Pairing.** Two users pair by scanning each other's QR code in person. The QR encodes identity and pairing material only — no mailbox address is fixed at pairing time, since reachability resolves dynamically whenever someone tries to reach that contact.
- **Messaging.** Users exchange text-only messages. How a message is delivered depends on both users' mailbox setup (see Section 5 and Section 8): instant live delivery when both are reachable directly, a single durable write when the recipient has a Server mailbox, or scheduled retry when the sender has a Server mailbox and the recipient doesn't.
- **Delivery status.** Messages show "sent," then "delivered" once acknowledged. A message that exhausts every delivery attempt is either marked "failed to deliver" in the thread (when a Server mailbox managed the attempt) or reverts to the sender's compose box unedited (an instant, unmanaged local-to-local attempt). There is no "read" indicator.
- **Mailbox.** Every user automatically has a **Local** mailbox — client-side, no setup, present from first launch. Any user may optionally add a single self-hosted **Server** mailbox for durable, asynchronous delivery. When both exist, Server is preferred and Local is the automatic fallback.
- **Recovery.** Identity is always recoverable from the 24-word recovery phrase alone. Contacts and message history are recovered automatically if the user has a Server mailbox, or via a separately maintained manual encrypted export the user manages themselves.
- **Manual backup export.** Any user, regardless of mailbox setup, can export an encrypted file containing their contacts and full message history, and import it during recovery on a new device.

**Explicitly out of scope for the MVP:** file/image/attachment sharing, group conversations, multi-device use (one identity per device), and background sync outside the foreground app.

## 4. Required Inputs

The product requires no phone number, email address, or account credentials of any kind. Inputs are:

- **Recovery phrase.** Generated automatically on first launch (24 words) and stored securely on-device; the user can view and record it at their own pace via Settings. Always sufficient to restore identity on a new device.
- **Camera access.** Required to scan a contact's QR code during pairing. Both parties must be physically present together.
- **Server mailbox connection details (optional).** If a user sets up their own self-hosted mailbox, they enter its connection information once to link the app to it.
- **Manual backup file (optional).** A user may export an encrypted contacts-and-history file and later supply it during recovery on a new device. Storing and locating this file is the user's own responsibility.

## 5. User Flows

**1. First launch.** The app silently generates a cryptographic identity and a 24-word recovery phrase, stored securely on-device. There is no mailbox to join and no invite code — the user is fully functional immediately, landing in an empty conversation list ready to pair with a contact. Nothing is explained about delivery mechanics up front; the first time it becomes relevant is if a send ever fails (see Flow 3), at which point a brief inline explanation appears in context.

**2. Pairing with a contact.** Two users, physically together, each open a QR screen and scan the other's code. The QR encodes identity and pairing material only — no mailbox choice is made at this point, since reachability resolves dynamically at send time (Server preferred, Local as automatic fallback). Because of this, adding, changing, or removing a Server mailbox later requires no re-pairing and no action from existing contacts.

**3. Sending a message.** What happens depends on which mailbox currently represents the recipient:
   - If the recipient has a Server mailbox, the sender's client writes to it directly — durable immediately, the recipient's phone doesn't need to be on. Shows "sent," then "delivered" once fetched and acknowledged.
   - If the recipient is Local-only and the sender is also Local-only, the sender's phone attempts a direct connection for ~15–20 seconds. Success delivers instantly; failure reverts the message to the sender's compose box, unedited, ready to retry.
   - If the recipient is Local-only but the sender has their own Server mailbox, the sender's client hands the message to their server, which attempts delivery to the recipient's phone on a configurable retry schedule (e.g., three attempts within the hour) rather than requiring the sender's phone to stay open. The sender sees only "sent" — no live retry status — until it resolves. If the schedule is exhausted, the message stays in the thread marked "failed to deliver."

**4. Receiving messages.** This differs by the recipient's own setup:
   - With a Server mailbox: opening the app syncs, fetching anything that arrived while away. Messages are acknowledged but **retained on the server** rather than deleted, to support future functionality built on that data. The app continues polling while foregrounded.
   - Local-only: there is no queue to sync. Anything sent while the user wasn't reachable simply never arrived — it failed silently on the sender's side, with no trace on the recipient's device. Opening the app only makes the user reachable going forward. This is inherent, not a bug, and is never surfaced with a nudge toward setting up a Server mailbox — that stays entirely self-discovered.

**5. Recovery on a new device.** Entering the recovery phrase always restores identity, since it's deterministic from the seed. If the user has a Server mailbox, their contacts and accumulated message history are restored automatically from it. If not, restoring contacts and message history depends entirely on the user having previously exported and saved a manual encrypted backup file themselves — there is no automatic recovery path for a Local-only user, by design (see Section 6).

## 6. Constraints

- **No group messaging, attachments, or multi-device support in the MVP.** Deferred, not abandoned.
- **No push notifications.** Delivery relies on direct connections and self-hosted infrastructure the user controls, never on Apple/Google push services.
- **Recovery depends on setup.** Identity is always recoverable from the seed phrase. Contacts and message history are only automatically recoverable if the user runs a Server mailbox; otherwise, recovery depends entirely on the user having manually exported and safely stored a backup file. A Local-only user who never exports a backup has no way to recover contacts or history if their device is lost — this is a deliberate incentive toward self-hosting, and must be stated plainly to the user, not buried.
- **No shared third-party mailbox operator exists in this design.** Every mailbox — Local or Server — is owned by its own user. The threat model no longer needs to account for a curious community operator; the residual risk is standard self-hosting exposure (a user's own Server mailbox being compromised by an external attacker), which the user accepts as the owner of that infrastructure.
- **Connectivity still depends on a real, shared third party, distinct from the mailbox layer above.** iroh's discovery (DNS/pkarr) and relay fallback default to n0's own public infrastructure (`*.relay.n0.iroh.link`, `dns.iroh.link`) — every device publishes its address there, and ciphertext may route through an n0-operated relay when a direct connection isn't available. This exposes endpoint identities and connection timing to n0, never message content, but it's a genuine exception to "no shared infrastructure" that removing the shared community mailbox does not resolve — see `docs/THREAT-MODEL.md`.
- **The threat model otherwise remains bounded** to commercial surveillance and casual network-level observation — not a global passive adversary.
- **Solo-maintainer project.** There is currently one maintainer, limiting response time to bug reports and security issues, and release pace, compared to a funded team. Reproducible builds and a published protocol spec are the stated mitigation.
- **Unbounded storage growth on Server mailboxes.** Since delivered messages are retained rather than deleted, a Server mailbox grows without a cap until a retention/quota policy is defined (see Open Items).

## 7. Interface Requirements

- **Onboarding** — identity and recovery phrase generated and stored silently; no invite code step; the user lands directly in an empty conversation list.
- **Conversation list** — paired contacts and their most recent message.
- **Chat view** — text messages; status shown as sent / delivered / failed-to-deliver. A failed instant local-to-local attempt reverts the message text to the compose box with a brief contextual explanation at that moment. No read receipts, no attachments.
- **Pairing screen** — dual-mode: display own QR code, scan a contact's QR code. No mailbox selection involved.
- **Manage Mailbox screen** — shows Local (always present, fixed) and, if configured, the user's single Server mailbox, with the ability to add, remove, or view its status.
- **Server mailbox setup screen** — enter connection details for a self-hosted mailbox the user is already running elsewhere.
- **Recovery phrase screen (Settings)** — view the recovery phrase on demand, at the user's own schedule.
- **Backup nudge** — a single, non-blocking, dismissible prompt shown once, encouraging the user to back up their recovery phrase. Does not recur, does not block functionality.
- **Backup export screen (Settings)** — manually export an encrypted contacts-and-message-history file; the user manages where it's stored.
- **Recovery screen** — enter the 24-word recovery phrase; optionally import a manual backup file afterward to restore contacts and message history when no Server mailbox is available to supply them automatically.

## 8. Technical Constraints

- **Platforms:** iOS and Android only (native mobile). No web or desktop client in scope.
- **App architecture:** React Native UI layer over a shared Rust core (all protocol/crypto logic lives in Rust, not TypeScript), bridged via `uniffi-bindgen-react-native`.
- **Transport:** iroh (QUIC over TLS 1.3), dial-by-public-key, with built-in hole punching and relay fallback that carries only ciphertext.
- **Encryption:** vodozemac (audited Matrix double-ratchet implementation) for message content; BLAKE2b/HKDF/XChaCha20-Poly1305 for tags, envelopes, and backups.
- **Local storage:** SQLCipher-encrypted SQLite on-device; database key held in the OS keychain/keystore, generated independently of the recovery phrase.
- **Mailbox model:** every user has an automatic, client-side Local mailbox. A user may additionally run a single self-hosted Server mailbox — the same `pm-node` Rust binary originally designed for this project, now deployed per-user rather than as shared community infrastructure. No shared, multi-tenant, community-operated instance exists anywhere in this design.
- **Delivery model:** not strictly pull-only. Delivery to a Local-only recipient is a direct, live connection attempt with a ~15–20 second timeout and no retry — closer to synchronous P2P than the original async pull design. Delivery to a recipient with a Server mailbox is a durable write, held until fetched. A sender with their own Server mailbox can hand off retry responsibility for reaching a Local-only recipient on a configurable schedule. No APNs/FCM push infrastructure is used in any path.
- **Server-side retention:** messages are retained on a Server mailbox after acknowledgment rather than deleted, to support planned future functionality built on that data. Retention/quota policy not yet defined (see Open Items).
- **Licensing/distribution:** MIT/Apache-2.0 dual-licensed, public repository, reproducible builds.
- **Dependency licensing preference:** where a suitable option exists, prefer MIT-licensed libraries when selecting new dependencies during implementation. This is a preference for new choices, not a re-litigation of already-locked dependencies with different permissive licenses — e.g., vodozemac (Apache-2.0) stays, since it was chosen specifically for its audited double-ratchet implementation.
- **Known dependency risk:** two core dependencies are early-stage — iroh 1.0 (shipped mid-2026) and `uniffi-bindgen-react-native`. Both are isolated behind thin internal abstractions so they can be replaced without rewriting protocol or UI code if needed.
- **Signed mailbox-pointer updates.** Any update to which mailbox currently represents a user (e.g., switching from Local to a self-hosted Server mailbox) must be published as a pointer record encrypted and addressed per contact pair, and signed by the user's identity key (IK). A contact's client must verify this signature before trusting a new mailbox address. This applies whether the change happens automatically or is user-initiated, and prevents an unauthenticated party from redirecting a contact's outgoing messages to a mailbox they control.

---

**Open items not resolved in this PRD** (carried forward for design/implementation decisions):

- **Retention/quota policy for Server mailboxes.** Messages are no longer deleted after acknowledgment, so storage grows unbounded without an explicit policy.
- **Rate-limiting/anti-abuse mechanics for the new delivery model.** The original protocol's slot-hash tag-chain scheme (and its associated message-window "stall" behavior) was built specifically to hide contact-graph information from a shared community operator. With no community operator in this design, whether that mechanism is still needed in its original form, or can be simplified, needs re-evaluation at the architecture level — not resolved here.
- **Manual backup file format and storage guidance.** The PRD assumes users will store this file themselves but doesn't yet specify its format, expected size (given it can include full message history), or guidance to reduce the risk of it being lost or exposed.
