# Layer 6 spec v0.1 — adversarial review, carriers lens

Judge: fable (carriers, adopted components, process shape, licensing). Date 2026-10-19. Draft at `23283ba`. Read-only; nothing run.

## Verdict

**amend-then-lock.** The adapter picks are the right adoptions and every run-tagged rule in §5 and §6 matches R01, R02 and R07. The defects are in the seams around them: the optional-component rules are not satisfiable as a set (zero disk, no runtime fetch, offline enable), a secret is rendered into a world-readable file, the MeshChatX catch-up and start-order rules lean on mechanisms the research and the supervisor do not have, and §14 applies the exact Reticulum-derivation pattern §12 rejects. None of these touches an owner decision; all are textual fixes plus two owner calls.

## Blockers

**B1. §2.1 is unsatisfiable as written.** Rule 2: "An `optional` one also has no payload on disk." Rule 3: the payload source is "the release payload or a pre-seeded cache … L1's 'no runtime downloads' stands." G7: "no component payload is on disk." A release payload and a cache are both on disk, so an `optional` component with no bytes on disk can never be enabled offline, and G7 passes only in a configuration where enable cannot work. The owner's F3 rule is about *running* weight ("if it's not running, it doesn't make our system heavier"), not disk. L1 already has the honest mechanism: install-time internet is guaranteed and the rns and presence closures are prefetched into root-owned `cache/packages/` (`10-bootstrap/README.md:53`, `install/packages.rs` `pip_closure`), venvs are built offline from it. **Fix:** define zero weight as runtime (no process, listener, loaded code, timer, fd, no `components/` or `venvs/<id>` tree); state that the signed manifest pins one lock per arch and libc plus the PBS tarball URL+sha256+size, that `111 install` prefetches them into `cache/packages/<component>/` (PBS only when `python3 < 3.11`), shows "N MB cached, not installed", and that G7 asserts the runtime baseline and "nothing installed", not "nothing on disk".

## Majors

**M1. `rpc_key` rendered into a 0644 file.** §6 puts the key in `secrets/rns-rpc-key` (0600) but renderer v2 writes it into `rns/config`; today `transport/config.rs:39` writes that file `0o644` under `rns/` (node111 0755, `10-bootstrap/README.md:116`). Since RNS 1.5.5 the key authorises `rnstatus --attach|--detach` over the control RPC, so any local user could detach the hub's radios. **Fix:** renderer v2 writes `rns/config` 0600, `rns/` and the child's client dir 0700; G4 asserts the modes.

**M2. Catch-up cursor assumes a global endpoint the research does not show.** §6 rule 5: "pages from [the highest row id] on every (re)connect, so a `111d` restart … loses nothing." R02 §3.7 and R07 check 4 show `after_id` only on `GET …/conversation/{dest}`; a first message from a peer in a conversation the adapter has never listed is not reached by per-conversation paging, and a store reset ("remove files" then re-enable) restarts ids below the kept cursor, so nothing pages until ids pass it. **Fix:** name the global or conversations-list endpoint and verify it in G3 (tag *src* until then), reset the cursor whenever the store is created or the highest id is below it, and keep the hash dedupe as the safety net.

**M3. `after = rnsd` is not a supervisor feature; respawns are independent.** §2 and §6 rule 1 promise the child starts only after rnsd owns the socket, including after crashes. `supervise.rs` has one back-off loop per child (`spawn_child`, `:298`), `stop_and_wait` is terminal (`:177`), and nothing gates one child's respawn on another's state. A double crash (OOM, reboot race) lets a respawning MeshChatX win the socket and partition the node with no error (R07 check 1b). §7 lists only `stdio` as the supervisor extension. **Fix:** specify the dependency gate (spawn or respawn only while rnsd is `Running` and `rnstatus` shows the shared instance serving), the post-spawn partition check after every child spawn with recovery (restart the child), and record it as the second supervisor extension or as bridge-module orchestration over stop + fresh `spawn_child`.

**M4. §14 derives Reticulum addressing arithmetic from docs, which §12 and R06 G6 reject for the RNode probe.** §14: "the sender's destination hash equals the one derived from that public key by the documented public addressing arithmetic … no Reticulum code." R02 §3.17 and R06 G6 say an LLM-agent reproduction from Reticulum material is the contested case; §12 drops the RNode probe bytes for exactly that reason and G5 forbids a stub "modelled on `RNodeInterface.py`". **Fix:** carry the LXMF destination hash inside the signed attestation (the child reports its own hash over its API; `transport_sig` by the Ed25519 seed already binds it) and compare hashes, no derivation; or record the exposure as an owner decision with the legal read of LICENSING §9.4.

**M5. `native` MeshChatX would ship Reticulum code.** §2.1 rule 5 leaves `delivery` to the owner; §15 says MeshChatX's closure "contains `rns` and `lxmf` as wheels … the same standing as the existing rns venv." That standing is "we ship only a requirements file with hashes, never Reticulum code; the user installs from PyPI" (LICENSING §3, §8). A `native` payload carrying the closure would redistribute `rns`/`lxmf`. **Fix:** state that for any `delivery` the `rns`/`lxmf` wheels come only from PyPI at install time (the rns-lock pattern) and never ride in a release payload, or name the legal read as the precondition for `native`.

**M6. The latest-upstream rule vs a second RNS copy.** D17 #12 and LICENSING §9.3: Reticulum is "always the latest upstream release." The MeshChatX venv carries its own `rns` resolved by its lock; after the 2027-01-01 sunset MeshChatX is frozen while RNS "releases weekly and its floors rise" (R02 §3.3). Two RNS versions on one node, one of them not latest, and a shared-instance protocol that may drift. **Fix:** both locks resolve `rns` to the same version at release time; a MeshChatX lock that cannot take the latest RNS is the stated trigger for option C; say so in §6 and §15.

**M7. Delta Chat consent copy is narrower than the core's behaviour.** §7: "The peer's core still decrypts incoming traffic to learn the sender; the opt-in decides what 111 stores." `deltachat-rpc-server` stores every contact's decrypted messages in its own account database regardless of 111's opt-in, so the plaintext the per-contact warning describes exists for non-bridged contacts too; §15.2 names MeshChatX's history as a residual but not this one. **Fix:** show the warning once per account ("this hub decrypts everything this account receives"), configure the core's `delete_device_after` for non-bridged chats or document the residual, and add the Delta Chat store to §15.2 and to "remove files".

**M8. Madmail's ports and the R-B promise.** §2 assigns Madmail "11233–11235 (LAN-bound)". Delta Chat clients reach a relay on IMAP/SMTP ports; whether the apps accept an IP-only, self-signed relay at all is "unverified" (§7) and the only evidence is *doc*. §1 nonetheless lists "Delta Chat + Madmail" in the first signed release, and `smp-server`'s LAN port (phones need it) is in no table. **Fix:** take the ports from the Madmail IP-only doc (standard ports need `CAP_NET_BIND_SERVICE`, which the unit grants and Termux cannot), add `smp-server`'s LAN port, and say what R-B ships if the attended gate fails (adapter present, offline relay marked unverified, public relay as the opt-in path).

## Minors

1. **§5 `want_ack` on channel text.** "for channel text the firmware forces it off" reads as "do not set it". R01 §3.7: the firmware clears the bit on air but keeps the local reliable record only when the client asked, and that record is what yields the implicit ack. Say `want_ack = true` on both; G2 asserts the record path.
2. **§4 "No timer retries" vs §5 pacing.** `RATE_LIMIT_EXCEEDED` ⇒ `Retry::Later` on "an adapter event", but the only correct trigger is the 2.1 s pacing timer (the bridge cannot see panel sends, only their `QueueStatus`). Name the pacing timer as the one timed retry.
3. **§5 crate feature trim unverified.** "less with its serial and BLE features trimmed" — R01 §3.16 measured the default feature set; whether such features exist is not read. Tag *unverified*; if absent, `serialport` (MPL-2.0) and btleplug link in.
4. **§5 wrapper vs the crate's state machine.** If the crate ever re-sends `want_config_id` (reconnect inside its API) and the wrapper drops it, the crate may wait for a `config_complete_id` that never comes. Add the G2 case "a dropped second `want_config_id` does not wedge the crate".
5. **§5, §11 two protobuf snapshots.** The crate's bundled snapshot generates the code; the KB region table is "generated from the pinned `config.proto`". Use one snapshot and confirm at pin time whether the crates.io package carries `.proto` files or pre-generated Rust (it changes the §15 row).
6. **G2's "settles R06 G7" is half.** A prost encode fixes the `Data` encoding (232/220, 231/219 are right from the shapes in R01 §3.4; the bitfield assumption is conservative because nanopb encodes a present `bitfield` even at 0). The 239/227 firmware limits are *src*; the hardware gate settles them. Say so.
7. **§6 auth window.** Between the child's bind and the setup POST the loopback API is open; on a Linux host that is inside the loopback-trust model (D16 a), on Termux it is not and Termux is excluded. State both so §15.9's "listener is not opened until auth is on" is not read as covering the child's own port.
8. **§6 `HOME=<root>`.** R07 §5: the child writes `~/.reticulum-meshchatx/logs/` even with `--storage-dir`, so state lands outside `bridge/lxmf/`; "remove files" must cover it.
9. **§6 posture consequences.** `enable_transport = yes` plus a LAN Backbone with `ifac = false` makes the hub route other people's Reticulum traffic, over its RNode too. Add one sentence to §15.8; the second AutoInterface and propagation role are already opt-in.
10. **§7 piped stdio.** A piped stdout that nobody drains blocks the child and the respawn loop; the supervisor must own a reader or hand the pipe over before the first write.
11. **§7 SimpleX "musl host".** Every L1 host is systemd and glibc; the only non-glibc host is Termux. Say "Termux" in the refusal copy.
12. **§2 `409 port_busy` for TCP ports.** `PortTable` already holds `127.0.0.1:11230` (`meshtastic/mod.rs:675`), so the claim works, but a port held by a foreign process is seen only as a failed bind; add the pre-bind listen check the panel listeners use.
13. **§5 Nearby parity.** "equal Nearby output" is undefined: the reader truncates names at 40 B (L5 §14), §5 says 24 B; fields differ. Define parity per schema field with the 24 B rule.
14. **§15 Madmail licence unread.** The row says "repo metadata only, file unread"; a redistributed component needs the file read before its pin, and its AGPL §13 source offer named.
15. **§15 notices.** `gen-notices.py --check` must hash all per-arch/libc MeshChatX locks and the PBS tarball pin, and a `python-licenses.txt` row per slim-closure package (35) is needed; name it in §18 Layer 1.
16. **§10 two proxy designs.** "keeps one server-side upstream session … (the alternative is a login page)" — pick at build is fine; say which one G6 lints.
17. **Labels.** The draft's gate "G6" collides with R06's "G6" rule; rename one.

## Owner questions

1. **Option C timing.** The first signed Layer 6 release will likely land after 2027-01-01; R02 §4 says decide C "before the carrier is built". Decide now, or accept shipping an archived upstream in R-B with C as the dated trigger (M6)?
2. **Where optional bytes live.** Install-time prefetch into `cache/packages/` (about 80 MB per hub for the slim closure plus PBS, never installed unless enabled), USB drop only, or a later runtime-fetch decision? B1 needs one answer.
3. **Natives.** Which components ship `native`, and may a native MeshChatX carry `rns`/`lxmf` wheels (legal read) or must those stay PyPI-only (M5)?
4. **Delta Chat if the Madmail gate fails:** stays in R-B online-only with the public relay opt-in, or moves to a later release (M8)?
5. **G6 exposure in §14:** attestation carries the hash (no derivation), or accept the derivation with a legal read (M4)?

## Sound

- Frame budgets: 232/220 text and 231/219 `PRIVATE_APP` follow from the `Data` shapes in R01 §3.4 (portnum 1 B vs 2 B varint, payload tag + 2 B length, 2 B bitfield) against 239/227.
- The shim's `disconnect` drop: `ToRadio` is one oneof, so the first byte `0x20` (field 4, varint) is exact.
- Allowlist content matches R01 and the reader's `ALLOWED_TORADIO` (`meshtastic_presence.py:91`); nonce-0 heartbeat, nonce never 1, `want_config_id` never 1/69420/69421.
- "One more client" is consistent: meshmud fans out to every downlink, the shim opens per-session downlinks, the downlink is claimed once for `meshmud`; the crate's `connect` takes any stream (R06).
- Port block 11231–11239 is free against L5 §2; 4242 LAN-only with LAN sharing.
- §6 rules 1–7 match R07 runs: start order, setup + PATCH + CSRF, stamp cost 0, auto-resend off, `lxmf.delivery`, field 16 marker, sticky `delivered`, no verify flag.
- rnsd posture v2 (`share_instance = yes`, `instance_name`, shared `rpc_key`, interface-less client dirs, `rnstatus --attach`) matches R02 §3.4–3.5 and R05.
- Licensing: GPL-3.0-only effect of F2 with the §14 question routed to a legal read; AGPL children and MPL core as separate processes; harness far end is MeshChatX, no RNS import; `StubAdapter`, stub meshmud, fake `rnodeconf`, MIT-doc MeshCore stub.
- The piped-stdio `ChildSpec` extension is buildable against `supervise.rs:37-45,333`; `ChildControl::restart` (`:209`) exists for the posture restart.
- Evidence tags are honest where it matters: app peers *doc*, echo and second AutoInterface *unverified*, meshtasticd "not yet evidence".
- Runtime zero weight holds: the Meshtastic client runs only while LIVE; children exist only after enable; the bridge core is ours.

## Method

Read the draft in full; D17, D19, D22, D23, D25; LICENSING §1, §3, §7–§9; L5 §13–§14; L1 install flow, layout, supervision, Layer 5 amendments; R01, R02, R03, R06, R07; OB F2–F4. Read `supervise.rs`, `ports.rs`, `meshtastic/{mod,meshmud,shim}.rs`, `transport/{mod,config}.rs`, `config.tmpl`, `panels/manifest.rs`, `services/mod.rs`, `install/packages.rs`, the presence reader's allow-list, and the generator headers. Checked each §5/§6 rule against the research line it cites, the arithmetic against the protobuf shapes, and each process-shape claim against the supervisor as built. No builds, no runs, nothing written outside this file.
