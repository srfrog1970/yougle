#!/usr/bin/env bash
# Simulates two separate NATed "home networks" using Linux network
# namespaces, then runs crates/pm-core/examples/nat_sim_peer twice — once
# in each namespace — to verify pm-core's real Local-to-local direct P2P
# delivery actually works when the two peers are NOT on a shared local
# subnet. See ./README.md for what this does and doesn't prove.
#
# Run as your normal user (NOT `sudo bash run.sh`) — this script calls
# `sudo` itself only for the specific networking commands that need root,
# so the cargo build and the log files it produces stay unprivileged. The
# first `sudo` call will prompt for your password; it's cached for the
# rest of the run.

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

NS_A=nat-sim-a
NS_B=nat-sim-b
SUBNET_A=10.200.1.0/24
SUBNET_B=10.200.2.0/24
HOST_IP_A=10.200.1.1
NS_IP_A=10.200.1.2
HOST_IP_B=10.200.2.1
NS_IP_B=10.200.2.2
VETH_MTU=1280
COORD_DIR=/tmp/nat-sim-coord
BINARY=target/release/examples/nat_sim_peer

log() { echo "[nat-sim] $*"; }

# Computed up front (before the cleanup trap is armed) so cleanup can
# always reference it safely even if something fails before the point
# where it'd otherwise be (re-)computed.
EGRESS_IF=$(ip route show default | awk '{print $5; exit}')

# --- Cleanup: always runs on exit (success, failure, or Ctrl-C), so a
# failed run never leaves namespaces/rules/log-in-progress state behind.
# Every command here is best-effort (|| true) since cleanup must not itself
# fail partway through and leave things half torn-down. ---
cleanup() {
    log "tearing down..."
    # sudo'd: the peers run as root (via `ip netns exec` under sudo), so a
    # plain unprivileged pkill can't touch them.
    sudo pkill -f "$BINARY --role a" 2>/dev/null || true
    sudo pkill -f "$BINARY --role b" 2>/dev/null || true
    sudo iptables -t nat -D POSTROUTING -s "$SUBNET_A" -o "$EGRESS_IF" -j MASQUERADE 2>/dev/null || true
    sudo iptables -t nat -D POSTROUTING -s "$SUBNET_B" -o "$EGRESS_IF" -j MASQUERADE 2>/dev/null || true
    sudo iptables -D FORWARD -s "$SUBNET_A" -d "$SUBNET_B" -j DROP 2>/dev/null || true
    sudo iptables -D FORWARD -s "$SUBNET_B" -d "$SUBNET_A" -j DROP 2>/dev/null || true
    sudo iptables -D FORWARD -s "$SUBNET_A" -j ACCEPT 2>/dev/null || true
    sudo iptables -D FORWARD -d "$SUBNET_A" -m state --state ESTABLISHED,RELATED -j ACCEPT 2>/dev/null || true
    sudo iptables -D FORWARD -s "$SUBNET_B" -j ACCEPT 2>/dev/null || true
    sudo iptables -D FORWARD -d "$SUBNET_B" -m state --state ESTABLISHED,RELATED -j ACCEPT 2>/dev/null || true
    sudo ip netns del "$NS_A" 2>/dev/null || true
    sudo ip netns del "$NS_B" 2>/dev/null || true
    sudo rm -rf "/etc/netns/$NS_A" "/etc/netns/$NS_B"
    log "torn down."
}
trap cleanup EXIT

# Clean up any stale state from a prior failed run before starting, so
# this script is safe to re-run.
sudo ip netns del "$NS_A" 2>/dev/null || true
sudo ip netns del "$NS_B" 2>/dev/null || true

log "building nat_sim_peer (release, unprivileged)..."
cargo build --release -p pm-core --example nat_sim_peer

log "egress interface: $EGRESS_IF"

# --- Step 0: loopback sanity check, no namespaces at all. Isolates "is
# the pairing/send logic correct" from "does the network topology work" —
# cheap and fast, so a doomed run fails here rather than after the full
# namespace setup. ---
log "loopback sanity check (no namespaces)..."
rm -rf "$COORD_DIR"
mkdir -p "$COORD_DIR"
"$BINARY" --role a --coord-dir "$COORD_DIR" > "$COORD_DIR/loopback-a.log" 2>&1 &
PID_A=$!
"$BINARY" --role b --coord-dir "$COORD_DIR" > "$COORD_DIR/loopback-b.log" 2>&1 &
PID_B=$!
wait "$PID_A" || true
wait "$PID_B" || true
if ! grep -q "delivered=ok" "$COORD_DIR/loopback-a.log" || ! grep -q "delivered=ok" "$COORD_DIR/loopback-b.log"; then
    log "FAIL: loopback sanity check didn't pass — see $COORD_DIR/loopback-{a,b}.log"
    log "(this means the pairing/send logic itself is broken, unrelated to namespaces — stopping here)"
    exit 1
fi
log "loopback sanity check passed."

# --- Namespaces, veth pairs, addressing, MTU, DNS. ---
log "creating namespaces..."
sudo ip netns add "$NS_A"
sudo ip netns add "$NS_B"

log "wiring veth pairs..."
sudo ip link add veth-a-host type veth peer name veth-a-ns
sudo ip link set veth-a-ns netns "$NS_A"
sudo ip addr add "$HOST_IP_A/24" dev veth-a-host
sudo ip netns exec "$NS_A" ip addr add "$NS_IP_A/24" dev veth-a-ns
sudo ip link set veth-a-host up mtu "$VETH_MTU"
sudo ip netns exec "$NS_A" ip link set veth-a-ns up mtu "$VETH_MTU"
sudo ip netns exec "$NS_A" ip link set lo up
sudo ip netns exec "$NS_A" ip route add default via "$HOST_IP_A"

sudo ip link add veth-b-host type veth peer name veth-b-ns
sudo ip link set veth-b-ns netns "$NS_B"
sudo ip addr add "$HOST_IP_B/24" dev veth-b-host
sudo ip netns exec "$NS_B" ip addr add "$NS_IP_B/24" dev veth-b-ns
sudo ip link set veth-b-host up mtu "$VETH_MTU"
sudo ip netns exec "$NS_B" ip link set veth-b-ns up mtu "$VETH_MTU"
sudo ip netns exec "$NS_B" ip link set lo up
sudo ip netns exec "$NS_B" ip route add default via "$HOST_IP_B"

log "configuring per-namespace DNS (WSL2's resolver is invisible inside a netns)..."
sudo mkdir -p "/etc/netns/$NS_A" "/etc/netns/$NS_B"
echo "nameserver 1.1.1.1" | sudo tee "/etc/netns/$NS_A/resolv.conf" > /dev/null
echo "nameserver 1.1.1.1" | sudo tee "/etc/netns/$NS_B/resolv.conf" > /dev/null

log "adding NAT (MASQUERADE) and cross-subnet isolation rules..."
sudo iptables -t nat -A POSTROUTING -s "$SUBNET_A" -o "$EGRESS_IF" -j MASQUERADE
sudo iptables -t nat -A POSTROUTING -s "$SUBNET_B" -o "$EGRESS_IF" -j MASQUERADE
# Belt-and-suspenders: no route between the two subnets exists anywhere,
# so this is currently unreachable in practice — but it makes the
# isolation an explicit, auditable rule instead of an emergent
# consequence of nobody adding a stray route later.
sudo iptables -A FORWARD -s "$SUBNET_A" -d "$SUBNET_B" -j DROP
sudo iptables -A FORWARD -s "$SUBNET_B" -d "$SUBNET_A" -j DROP
# Docker manages this host's FORWARD chain and sets its default policy to
# DROP, only carving out ACCEPT for its own traffic (DOCKER-USER/
# DOCKER-FORWARD) — confirmed via `iptables -L FORWARD -n -v`. Without
# these, traffic from the new veth subnets hits that default DROP before
# ever reaching the MASQUERADE rules above, regardless of them being
# correct. Scoped to only these two subnets (source for outbound, dest +
# ESTABLISHED/RELATED for the return leg) rather than touching the
# chain's policy or anything Docker-related.
sudo iptables -A FORWARD -s "$SUBNET_A" -j ACCEPT
sudo iptables -A FORWARD -d "$SUBNET_A" -m state --state ESTABLISHED,RELATED -j ACCEPT
sudo iptables -A FORWARD -s "$SUBNET_B" -j ACCEPT
sudo iptables -A FORWARD -d "$SUBNET_B" -m state --state ESTABLISHED,RELATED -j ACCEPT

# --- Isolation check: confirm the two namespaces genuinely cannot reach
# each other directly. Note `ip route get` isn't the right test here — it
# always succeeds by resolving via the default route, regardless of
# whether the destination is actually reachable (that's just what "route
# get" means: which route *would* be used, not whether the packet
# survives past it). An actual reachability probe is what proves the
# FORWARD DROP rule above is doing its job. ---
log "verifying isolation (nat-sim-a should NOT be able to reach nat-sim-b directly)..."
if sudo ip netns exec "$NS_A" ping -c1 -W2 "$NS_IP_B" > /dev/null 2>&1; then
    log "WARNING: nat-sim-a can reach nat-sim-b directly — isolation is NOT working as intended"
else
    log "confirmed: nat-sim-a cannot reach nat-sim-b directly."
fi

# --- General connectivity diagnostic: isolates "DNS resolution is
# broken" from "nothing reaches the internet at all" from inside the
# namespace, before blaming pm-core/iroh for either. ---
log "connectivity diagnostic from inside nat-sim-a..."
if sudo ip netns exec "$NS_A" ping -c1 -W3 1.1.1.1 > /dev/null 2>&1; then
    log "  raw IP reachability (ping 1.1.1.1): OK"
else
    log "  raw IP reachability (ping 1.1.1.1): FAILED — NAT/forwarding/MTU problem, not a DNS problem"
fi
if sudo ip netns exec "$NS_A" getent hosts dns.iroh.link > /dev/null 2>&1; then
    log "  DNS resolution (dns.iroh.link): OK"
else
    log "  DNS resolution (dns.iroh.link): FAILED"
fi

# --- Optional, non-fatal hairpin-NAT precheck. This machine's own NAT
# chain (WSL2 -> Hyper-V vSwitch -> likely a home router) may or may not
# support hairpin/loopback for "two internal hosts, one router's own WAN
# IP" — outside anything this script's iptables rules can influence. A
# failed precheck means a relay-only result below is inconclusive on
# direct/hole-punched connectivity specifically, not a failure of this
# test — see README.md. ---
log "hairpin-NAT precheck (informational only, does not affect pass/fail)..."
PUB_IP=$(curl -s --max-time 5 https://ifconfig.me || echo "")
HAIRPIN_OK=no
if [ -n "$PUB_IP" ]; then
    sudo ip netns exec "$NS_B" timeout 3 nc -u -l -p 9999 > "$COORD_DIR/hairpin.recv" 2>/dev/null &
    NC_PID=$!
    sleep 0.5
    sudo ip netns exec "$NS_A" sh -c "echo hairpin-test | timeout 2 nc -u -w2 $PUB_IP 9999" 2>/dev/null || true
    wait "$NC_PID" 2>/dev/null || true
    if grep -q "hairpin-test" "$COORD_DIR/hairpin.recv" 2>/dev/null; then
        HAIRPIN_OK=yes
    fi
else
    log "could not determine this host's public IP — skipping hairpin precheck"
fi
log "hairpin-NAT precheck result: $HAIRPIN_OK"

# --- The actual test: one nat_sim_peer process per namespace. Fresh
# coordination dir — the loopback check above used the same default
# --coord-dir, and its leftover pairing files/sqlite stores (a different
# random identity each run) must not be reused here: a stale .json would
# skip real pairing entirely, and reopening its .sqlite with this run's
# different fresh seed would fail as a wrong-key SQLCipher error. ---
rm -rf "$COORD_DIR"
mkdir -p "$COORD_DIR"
log "launching peers inside their respective namespaces..."
sudo ip netns exec "$NS_A" env RUST_LOG=iroh=info,pm_core=debug \
    "$PWD/$BINARY" --role a --coord-dir "$COORD_DIR" --store "$COORD_DIR/a.sqlite" \
    > "$COORD_DIR/a.log" 2>&1 &
PEER_A_PID=$!
sudo ip netns exec "$NS_B" env RUST_LOG=iroh=info,pm_core=debug \
    "$PWD/$BINARY" --role b --coord-dir "$COORD_DIR" --store "$COORD_DIR/b.sqlite" \
    > "$COORD_DIR/b.log" 2>&1 &
PEER_B_PID=$!

log "waiting for both peers (up to 90s)..."
SECONDS=0
DEADLINE=90
while kill -0 "$PEER_A_PID" 2>/dev/null || kill -0 "$PEER_B_PID" 2>/dev/null; do
    if [ "$SECONDS" -ge "$DEADLINE" ]; then
        log "timed out waiting for peers — killing them"
        sudo kill -9 "$PEER_A_PID" "$PEER_B_PID" 2>/dev/null || true
        break
    fi
    sleep 1
done
# `set -e` would abort the script the instant `wait` returns nonzero
# (a peer exiting with an error is an expected, handled outcome here, not
# a script bug) — `|| PEER_x_EXIT=$?` catches that without tripping it.
PEER_A_EXIT=0
wait "$PEER_A_PID" 2>/dev/null || PEER_A_EXIT=$?
PEER_B_EXIT=0
wait "$PEER_B_PID" 2>/dev/null || PEER_B_EXIT=$?

echo
log "=== a.log RESULT lines ==="
grep RESULT "$COORD_DIR/a.log" || echo "(none)"
log "=== b.log RESULT lines ==="
grep RESULT "$COORD_DIR/b.log" || echo "(none)"
echo

if [ "$PEER_A_EXIT" -eq 0 ] && [ "$PEER_B_EXIT" -eq 0 ]; then
    log "PASS: both directions delivered successfully across the simulated NAT boundary."
    log "(hairpin-NAT precheck was: $HAIRPIN_OK — see README.md for how to read this alongside the result)"
else
    log "FAIL: see $COORD_DIR/a.log and $COORD_DIR/b.log for details."
    exit 1
fi
