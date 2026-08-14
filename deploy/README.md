# deploy

Running your own `pm-node` — a single-tenant Server mailbox, per
`docs/PRD.md`'s "no shared community infrastructure" design. Two ways to
run it: Docker (`Dockerfile` + `docker-compose.yml`) or directly as a
systemd service (`pm-node.service`). Pick whichever fits the machine
you're running it on (a Raspberry Pi, a small VPS, a home server, etc.).

Status: both paths verified for real, not just written and assumed
correct — built and ran via `docker compose` (real relay connection, a
printed pasteable address, `restart: unless-stopped` correctly leaving an
explicitly-stopped container stopped) and installed and ran as an actual
systemd unit (`enabled`+`active`, `DynamicUser=yes` confirmed running as
an unprivileged `pm-node` user rather than root, `Restart=always`
confirmed set). Persistent storage (`PM_NODE_DATA_DIR`) has also been
verified for real on both paths: with it set, a message written to a
running node survives `docker compose restart` and, separately, a real
`systemctl restart pm-node` (with `/var/lib/pm-node` confirmed owned by
the `DynamicUser`-assigned uid, not root) — not just a passing unit test.
Not yet verified: an actual in-process crash triggering systemd's/Docker's
auto-restart (the minimal runtime image has no `kill`/`pkill` to trigger
one from inside; the policies themselves are confirmed correctly
configured), and real connectivity between two genuinely separate home
networks/NATs rather than one host reaching the public relay.

## 1. Get your keys from the app

`pm-node` needs two values that identify it as *your* Server, both
derived from your own seed phrase: open the app, go to **Manage Mailbox
→ "Set up your own node"**, and tap to reveal. You'll see two lines
already formatted as:

```
PM_NODE_MAILBOX_KEY=<64 hex characters>
PM_NODE_TRANSPORT_KEY=<64 hex characters>
```

Copy both — you'll paste them as-is into whichever `.env` file below.
These identify your node as belonging to *you*; treat them with the same
care as the node's own configuration (not as sensitive as your 24-word
recovery phrase, which can restore your whole identity, but anyone who
has both these values *and* can reach the resulting node could act as
its owner).

## 2a. Run it with Docker

```
cd deploy
cp .env.example .env
# paste your two keys into .env
docker compose up -d
docker compose logs -f   # watch for the address it prints
```

## 2b. Run it with systemd (no Docker)

Requires Rust and the same build dependencies `pm-store`'s vendored
SQLCipher/OpenSSL build needs elsewhere in this project (a C toolchain,
`pkg-config`, `perl`) — see the repo root `README.md`.

```
cargo build --release -p pm-node
sudo cp target/release/pm-node /usr/local/bin/pm-node
sudo mkdir -p /etc/pm-node
sudo cp deploy/.env.example /etc/pm-node/pm-node.env
sudo chmod 600 /etc/pm-node/pm-node.env
sudo $EDITOR /etc/pm-node/pm-node.env   # paste your two keys in

sudo cp deploy/pm-node.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now pm-node
sudo journalctl -u pm-node -f   # watch for the address it prints
```

## 3. Connect your phone to it

However you ran it, `pm-node` prints its own address on startup — a long
opaque string, not a hostname or IP you'd type by hand. Copy that string
back into the app: **Manage Mailbox → Add Server mailbox**, paste it,
save. Any message sent to you now durably lands on your own node instead
of requiring your phone to be reachable live; if a Local-only contact
tries to reach *you* directly while your phone's offline, your node picks
up the retry on your behalf too (`docs/PRD.md` Flow 3's third case).

## Networking

No manual port-forwarding should be needed for the common case: `pm-node`
uses iroh's built-in NAT traversal (UPnP/hole-punching) with a relay
fallback, the same transport already used everywhere else in this
project. If your specific network setup needs a fixed port opened on your
router, that's not supported yet — `pm-node` binds an ephemeral port today
(its network *identity* is fixed by `PM_NODE_TRANSPORT_KEY`, but which
port it lands on isn't), which is also why the Docker Compose file uses
host networking rather than a published port mapping.

## Data

`pm-node`'s mailbox storage (stored messages, the backup blob, and the M7
retry-delivery queue) is SQLCipher-encrypted at rest, keyed by a value
derived from your `PM_NODE_MAILBOX_KEY` — never a copy of that value
itself. Whether it persists across a restart depends on `PM_NODE_DATA_DIR`,
which both deployment paths here already set up for you:

- **Docker**: `docker-compose.yml` mounts a named volume at `/data`, and
  `.env.example` has `PM_NODE_DATA_DIR=/data` ready to uncomment — do that
  before your first `docker compose up -d` if you want durability from the
  start (changing it later just means today's already-running in-memory
  data doesn't carry over, not that anything breaks).
- **systemd**: `pm-node.service` sets `PM_NODE_DATA_DIR` for you via
  `StateDirectory=pm-node`, which systemd resolves to `/var/lib/pm-node` —
  nothing to configure.

Leaving `PM_NODE_DATA_DIR` unset (or commenting it back out for Docker)
runs fully in-memory, matching this project's original default: a
restart clears anything not yet fetched by its owner or delivered by a
pending retry.
