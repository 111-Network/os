# Reaching a hub through a tunnel

Layer 7 §6.6 item 3. Two hubs that have no public address between them can still pair if the owner puts a tunnel under them. This page says how, and
what the tunnel does not do. It is a how-to for programs you install yourself: 111 ships none of them, adds no component and runs nothing for them.

## Never required

Pairing works over the same LAN, over one public address (a hub with a port forward, a VPS), or through a tunnel. A tunnel is the third way.
Nothing in 111 needs one, nothing in the base install depends on one, and nothing here starts unless you start it.

## What a tunnel is for

A tunnel is a pipe between two always-on ends. It gives each hub an address the other can dial. It does not store anything: if both hubs are
intermittent (two 5A islands), the tunnel is down whenever either end is, and nothing waits in it. The carrier (§6) is what holds messages for a hub
that is off. Use a tunnel for hubs that are up together; use a carrier for hubs that are not.

## Two adopted programs

| Program | What it gives | Cost to know about |
|---|---|---|
| WireGuard (kernel module, or `wireguard-go`) | an encrypted point-to-point interface, keys exchanged by hand | one end needs a reachable UDP endpoint; a kernel interface needs privilege (on a Termux phone, root) |
| Headscale (self-hosted control server) with Tailscale clients | many hubs on one private network, with NAT traversal | the control server needs a public host and is another thing to keep running; `tailscaled` is a resident process on every node that uses it |

Both are separate programs under their own licences: 111 neither ships nor links them, and their licences are their own, not recorded in this repo.
Neither is a Layer 7 dependency.

## Pairing inside a tunnel

1. Bring the tunnel up on both hubs and check that each can reach the other's tunnel address.
2. On each hub, open Settings → Layer 7 → "Pair with another hub" (or run `111 fabric pair --show`), and show the pairing bundle (§4.6 step 1). At
   least one bundle must carry an address the other hub can dial through the tunnel, on port 11183 (§4.6 step 3: one dialled address suffices); a
   LAN address the other hub cannot reach is no use. No control for choosing that address is pinned yet. For the Layer 5 hub offer the daemon takes
   the address from the fabric's `offer_host` when one is configured, else the hub's first LAN IPv4 (`offer_address` in
   `1-bootstrap/crates/node111/src/daemon/fabric/mod.rs`), and the production configuration sets no `offer_host`. Read the address the bundle
   shows before you hand it over, and do not pair through the tunnel until it is the tunnel's address.
3. Scan or paste the other hub's bundle and pair as usual (§4.6): the two hubs meet once over `tcp-lan` through the tunnel.
4. **Mark the pair `lan = true`:** the pairing request's `lan` field (`PairRequest` in `4-interface/api/openapi.yaml`; Layer 7 §4.6; the Settings
   control arrives with the pairing rows of the Layer 7 build). The tunnel already encrypts the path, so the pair runs plain `tcp-lan` inside it
   (§4.1 rule 1), with no second TLS layer and no "this link cannot be encrypted" warning. Without the mark a pair whose address is not private
   runs TLS 1.3 with raw public keys. The range `100.64.0.0/10`, which Tailscale and Headscale use, counts as private for that rule; set the field
   anyway, so the record says what it is.

The mark is yours to make and 111 cannot check it. If the address is not inside the tunnel, a `lan = true` pair sends the hub-to-hub frames in clear
across whatever path the address takes; the sealed message contents stay sealed, but the path sees which hubs talk, when and how much (§4.7).

## Logging: read this before a Tailscale client goes on a node

A stock Tailscale client logs to its vendor unless it is started with the environment variable `TS_NO_LOGS_NO_SUPPORT` set. That is outbound traffic
the node's owner did not ask for, and the Layer 0 gate for adopted projects counts it as a telemetry failure. Run the client with the variable set, or
do not use it. Under a Headscale control server the client's log upload is reported to be off already. Both statements come from a search
snippet a scout read (R07g1 §3 item 8), not from the vendors' documentation or a test; check them on your own node before relying on them.

A WireGuard interface has no vendor to log to.

## What stays the same

Messages are sealed end to end before they reach any pipe (Layer 5). A tunnel changes who can reach a hub and what the path sees; it adds nothing to
who can read a message. The link key (§4.2) and the pairing bundle are the same with or without a tunnel.
