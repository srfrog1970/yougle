# nat-sim

Simulates two separate NATed "home networks" locally, to verify
`pm-core`'s real Local-to-local direct P2P delivery (the M6 feature)
actually works when two peers are **not** on a shared local subnet — every
prior test of this feature ran both sides in the same OS process on the
same network, which never exercised real NAT traversal, discovery, or
relay fallback at all.

## What this proves, and what it doesn't

Two Linux network namespaces (`nat-sim-a`, `nat-sim-b`), each with its own
private subnet, NATed out independently, with no route between them —
structurally, not just by firewall rule. Two real `pm_core::Client`s, one
per namespace, pair and exchange one message each way. This proves:

- End-to-end message delivery genuinely works via `pm-core`'s real direct
  delivery path when the two peers can't reach each other on a shared
  local subnet — they have to go out through NAT and come back in via
  iroh's real discovery/relay infrastructure.
- The isolation is real: `run.sh` checks that `nat-sim-a` has no route to
  `nat-sim-b` at all.

**What it can't prove**: both namespaces still share this one machine's
single real upstream internet connection, which is itself already
double-NATed (WSL2 → Hyper-V vSwitch → likely a home router) before
reaching the real internet. A true *direct*, hole-punched connection
between the two simulated peers would require every NAT layer in that
chain — including ones this script's `iptables` rules have zero control
over — to support NAT hairpin/loopback (two internal hosts reaching each
other via their router's own WAN-facing address), which plenty of real
routers simply don't support.

**So: a relay-only result is inconclusive on hole-punching specifically,
not a failure.** It can't distinguish "iroh's hole-punch logic doesn't
work" from "this specific host's NAT chain doesn't support hairpin
loopback." The thing this test actually, unambiguously proves either way
is that delivery succeeds end to end across genuinely non-local paths —
which is real, valuable signal on its own.

`run.sh` runs a cheap, best-effort hairpin-NAT precheck before the real
test and reports it alongside the result:

| Hairpin precheck | Result | How to read it |
|---|---|---|
| succeeded | delivered=ok (either path) | Solid signal either way — hairpin loopback works on this host, so a relay-only outcome would be worth a closer look. |
| failed | delivered=ok | Expected and fine — relay fallback proved the pipeline works; direct/hole-punch specifically is inconclusive on this host, not broken. |
| either | delivered=err | A real failure — dig into `a.log`/`b.log`. |

A genuinely separate second physical device on a genuinely separate
network remains the real, final confirmation of true cross-network
hole-punching. This test de-risks that step — it doesn't replace it.

## Running it

```
cd deploy/nat-sim
./run.sh
```

Run as your normal user, **not** `sudo bash run.sh` — the script calls
`sudo` internally only for the specific namespace/iptables commands that
need root, so the `cargo build` step and the resulting logs
(`/tmp/nat-sim-coord/{a,b}.log`) stay owned by you, not root. The first
`sudo` call prompts for your password; it's cached for the rest of the
run.

It always tears everything down on exit (success, failure, or Ctrl-C) —
namespaces, veth pairs, iptables rules, `/etc/netns/` entries — so it's
safe to re-run.

## What it actually does

1. Builds `crates/pm-core/examples/nat_sim_peer` (release, unprivileged).
2. Runs a loopback sanity check first (both roles, no namespaces at all)
   — isolates "is the pairing/send logic correct" from "does the network
   topology work," so a doomed run fails fast rather than after the full
   namespace setup.
3. Creates the two namespaces, veth pairs, addressing, and per-namespace
   DNS (WSL2's own resolver is invisible inside any `ip netns exec`
   namespace — this is set up explicitly, not something to debug as a
   NAT/iptables problem if it's missing).
4. Adds NAT (`MASQUERADE`) so each namespace's traffic can reach the real
   internet, plus explicit `FORWARD ... DROP` rules between the two
   subnets (belt-and-suspenders — no route between them exists anyway).
5. Runs the hairpin-NAT precheck described above.
6. Launches one `nat_sim_peer` process inside each namespace
   (`--role a`/`--role b`), which pair for real and exchange one message
   each way through `pm_core::Client`'s real direct-delivery path — role A
   sends first and confirms delivery, then role B (having now received
   A's message and established its side of the one shared Olm session for
   this pair) replies. This ordering matters: Olm has exactly one session
   per pair, established lazily by whichever side sends first — both
   sides trying to be "first" simultaneously would create two independent,
   mismatched sessions instead of reusing one, which isn't a real-world
   scenario (a reply always comes after receiving something first).
7. Reports `RESULT` lines from both peers and an overall PASS/FAIL.
