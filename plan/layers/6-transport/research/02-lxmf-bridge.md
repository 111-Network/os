# Layer 6, lens 2 — the Reticulum/LXMF bridge (2026-10-19)

Question: how does a 111 node become an LXMF endpoint (Messages category "Reticulum") through the rnsd child 111d already runs,
without importing RNS or LXMF in our code? Facts below were fetched on 2026-10-19; fetched facts override the KB. Anything not read
from an upstream file is marked (unverified). Code lives in `/tmp` scratch only; nothing in the repo was changed except this file.

## 1. Summary

- **Pick: MeshChatX 4.10.7 (0BSD AND MIT) as a headless supervised child beside rnsd**, attached to the shared instance. 111d talks to its
  loopback REST + WebSocket (`POST /api/v1/lxmf-messages/send`, `GET /api/v1/lxmf-messages/conversation/{hash}`, WS `/ws` event `lxmf_message`).
  No rule change. The same child is the hosted panel P7, which gives the owner "the other network's UI available fully" for free.
- **The pick has a hard expiry.** MeshChatX's README says development and releases end on 2027-01-01 (v5.0.0 last, repo archived, "fork/rebranding
  ready"). Its REST API is the UI's internal API (undocumented), cannot set a title or arbitrary LXMF fields, and exposes no
  signature-valid flag. Build the adapter behind a narrow carrier trait so a successor fork or Option C can replace it.
- **lxmd alone cannot do the job**: its hook is inbound-only, the saved file carries no signature-valid flag, there is no way to send, and its
  default rejects inbound resource transfers above 1,000 B. Keep it only as an optional propagation-node role.
- **Option C (a ~300-line sidecar that imports RNS/LXMF) is technically the sturdiest**, but it is a D17 #12 rule change, presented in section 4
  with its license consequences. Not the default.
- **rnsd posture**: `share_instance` must become `yes` (it is Reticulum's own default; `config.tmpl:9` overrides it to `no`), plus a shared
  `rpc_key` and separate interface-less client config dirs. `enable_transport` stays `no`. A phone-facing `BackboneInterface` listener with IFAC is optional.
- **LXMF has no channels** (no group addressing; `FIELD_GROUP` 0x0B exists with no spec). "Send the same message to Reticulum" therefore needs an
  owner decision on what the target is (open question 2).

## 2. Candidates

| Name | Upstream | License (verified?) | Language / runtime | Maturity | Fit | Why |
|---|---|---|---|---|---|---|
| Reticulum `rnsd` (RNS 1.5.7) | github.com/markqvist/Reticulum | Reticulum License (yes, `LICENSE`, 2016-2026) | Python 3.7+, pure wheel | PyPI 2026-10-05; 8 releases in 6 weeks; public mirror, "no support" | adopt | Already our connected service; the only source of Reticulum behaviour. |
| LXMF 1.2.1 + `lxmd` | github.com/markqvist/LXMF | Reticulum License (yes, same text, 2020-2025) | Python, pure wheel | PyPI 2026-10-13 | bridge (partial) | Needed in the venv (MeshChatX requires it). `lxmd` gives inbound hook + propagation node, no send. |
| **MeshChatX 4.10.7** | github.com/Quad4-Software/MeshChatX | 0BSD (Quad4 parts) AND MIT (MeshChat parts) (yes, `LICENSE`) | Python >=3.11, 22 MB wheel with UI | 25 PyPI releases 2026-05-04..10-11; 159 stars; **ends 2027-01-01** | **bridge (pick)** | Full LXMF stack + web UI + REST/WS; attaches to a shared instance; headless flags. Sunset and internal API are the risks. |
| MeshChat (original) 2.4.0 | github.com/liamcottle/reticulum-meshchat | MIT (yes) | Python + Node build | 2.4.0 | reference | Fallback only: no auth, needs Node + Rust on Termux, no WS Origin handling (KB). |
| Option C sidecar | would be ours | Reticulum License governs the running combination | Python, rns+lxmf pure wheels | n/a | reference | Contingency; needs owner rule change (section 4). |
| LXMFy 2.1.1 | github.com/Quad4-Software/LXMFy | 0BSD (yes, header) | Python >=3.11 | PyPI 2026-09-23 | reject | Command-bot framework; external-script cogs reply with stdout to `/command` messages only, cannot relay free text or send to arbitrary hashes. |
| LXMF-Tools (SebastianObi) | github.com/SebastianObi/LXMF-Tools | MIT (yes) | Python | 72 commits, "beta", undated | reference | Shows the distribution-group fan-out pattern and MQTT/Meshtastic/Matrix bridges; own custom field ids, no stable API. |
| NomadNet 1.4.4 | github.com/markqvist/NomadNet | GPL-3.0 (yes, header) | Python TUI/daemon | PyPI 2026-09-30 | reject (messaging) | No message API. Keep as a page-node peer in the KB. |
| rnsh 0.1.7 | github.com/markqvist/rnsh | MIT (yes, Aaron Heise 2023) | Python | RNS 1.3.9 shipped a critical rnsh fix | reject | A shell; violates "fixed shell-free tools". |
| Sideband | github.com/markqvist/Sideband | CC BY-NC-SA 4.0 (yes) | Python/Kivy | active | reject | Non-OSI; it is the main peer we talk TO, never ship. |
| Columba | github.com/torlando-tech/columba | MPL-2.0 (yes, `LICENSE.md`) | Kotlin + py/kt backends | 1.2k stars | reference | Android peer; dirs `rns-api/rns-ipc` exist, external use unverified. |
| Leviculum (`lnsd`) | codeberg.org/Lew_Palm/Leviculum | AGPL-3.0-or-later (page only; unverified in file) | Rust | commit 2026-10-18; "not yet production-ready" | reject | Not upstream; breaks D17 #12 "latest upstream"; upstream calls such copies a licence problem. |
| `lxmf-wire` (LXMF-rs) 0.14.0 | github.com/FreeTAKTeam/LXMF-rs | EPL-2.0 OR GPL-2.0-or-later (crates.io; `LICENSE` file EPL-2.0) | Rust | 2026-10-11 | reject | Same reason; also a reimplementation. |
| rrcd (Reticulum Relay Chat hub) | github.com/kc1awv/rrcd | MIT (yes) | Python | 61 stars | reference | The nearest thing to channels over Reticulum; a separate protocol, not LXMF. Out of v1. |
| send-lxmf (jahway603 fork) | zff.dev/jahway603/send-lxmf (Codeberg mirror) | none found | Python | last commit 2026-05-15 | reject | No license, so it cannot be adopted. |
| Reticulated simulator | github.com/RFnexus/Reticulated | Unlicense (yes) | Python + web UI | 14 commits | reference | Possible netns-free test aid; web-UI driven, headless use unclear. |

## 3. Findings that change the plan

1. **Versions are newer than the KB.** PyPI: RNS 1.5.7 (2026-10-05), LXMF 1.2.1 (2026-10-13), NomadNet 1.4.4 (2026-09-30), MeshChatX 4.10.7
   (2026-10-11); all pure `py3-none-any` wheels. KB `projects/reticulum.md` says RNS 1.3.5 / LXMF 1.0.1 and `projects/lxmf.md` says LXMF 1.2.0:
   all stale. Sha256 for the lock: rns wheel `0e4aaf8c…faddf`, lxmf wheel `4bd95325…31440` (https://pypi.org/pypi/rns/json, https://pypi.org/pypi/lxmf/json).
2. **GitHub is a lagging public mirror, not the source.** `MIRROR.md` (commit 2025-12-27): "public mirror … no support, no responses to issues";
   development is private. The LXMF mirror is at 1.2.0 (last commit 2026-09-30) while PyPI has 1.2.1; RNS tag 1.5.5 (2026-09-29) vs PyPI 1.5.7.
   The latest-upstream pin must come from PyPI hashes, which already matches the hash-locked venv pattern. Releases are also signed (`rsm`/`rsg`, signer
   `bc7291552be7a58f361522990465165c`, Changelog 1.5.5) — a verification option for the updater.
3. **MeshChatX sunsets 2027-01-01** (https://github.com/Quad4-Software/MeshChatX README "Project status"). v5.0.0 will be the last release. RNS releases weekly and
   its floors rise (MeshChatX 4.10.7 requires `rns>=1.5.7`, `lxmf>=1.2.0`), so a frozen MeshChatX will eventually meet an RNS it was not tested with.
4. **Reticulum's default is `share_instance = Yes`; our template sets `no`** (`1-bootstrap/crates/node111/assets/rns/config.tmpl:9`). Any LXMF program
   started with default config tries to *become* the shared instance, and falls back to client only when the bind fails
   (`RNS/Reticulum.py` ~lines 405-460). Consequences: (a) rnsd must be `yes` and up before any child starts, and children must be restarted when rnsd restarts;
   (b) a client with a different config dir derives a different `rpc_key` (hash of its own transport identity, line 362), so RPC (stats, RSSI) fails
   unless both configs carry the same `rpc_key`; (c) the shared socket is an abstract AF_UNIX name `\0rns/<instance_name>` (data) and `…/rpc` on Linux and
   Android, or 127.0.0.1:37428/37429 with `shared_instance_type = tcp` (`LocalInterface.py` lines 68, 388). Abstract sockets are per network namespace, which is
   exactly what the netns test plan needs. Docs: https://reticulum.network/manual/using.html (`ret-using.rst` lines 76-125).
5. **Live interface changes exist since RNS 1.5.5**: `rnstatus --attach/--detach/--reload <name>` over the control RPC; interface management is on by default
   (`enable_interface_management`, `Reticulum.py` lines 260, 553). 111d currently restarts rnsd on every netlink change
   (`src/daemon/transport/mod.rs:170-186`), which would drop every LXMF client session. Adding a new section to the config and attaching it by name avoids that.
6. **lxmd facts** (`LXMF/Utilities/lxmd.py`, identical in the 1.2.1 wheel): `on_inbound` runs `program "<path>"` with stdout/stderr discarded (lines 295-305). The file is
   a msgpack container `{state, lxmf_bytes, transport_encrypted, transport_encryption, method}` (`LXMessage.packed_container`): **no `signature_validated`
   / `unverified_reason`**, although the router delivers unverified messages to callbacks (`LXMRouter.lxmf_delivery`, lines 1906-1987: only blackhole, ignore and
   stamp checks drop). No display names, no announces, no send path. Defaults: delivery stamp cost 12 announced but not enforced (lines 90-93);
   `delivery_transfer_max_accepted_size = 1` means 1,000 B (`LXMRouter.delivery_resource_advertised`, line 2048), so any inbound message that needs a Resource is refused; propagation
   storage 500 MB, per-transfer 256 KB, `autopeer` on. The identity is a file in lxmd's config dir (`identity`), created on first run.
7. **MeshChatX API, read from source** (`meshchatx/src/backend/http/routes/lxmf.py`, `auth.py`; `docs/en/installation.md`): send body
   `{"lxmf_message": {"destination_hash", "content", "fields"}, "delivery_method"}`; fields accepted are image, audio, file attachments, telemetry, commands and an
   `app_extensions` dict (LXMF field 16); **no title, no arbitrary field ids**. Message JSON has `hash, source_hash, destination_hash, is_incoming, state, progress, method,
   delivery_attempts, title, content, fields, timestamp, rssi, snr, quality` — no verification flag. Pageable `GET …/conversation/{dest}?count&after_id&order`. WS pushes
   `lxmf_message`, `lxmf_message_created`, `announce` (`meshchat.py` ~9143, 10319, 11079). Auth: `--auth` password (bcrypt, cookie session), CSRF token from
   `GET /api/v1/auth/csrf` on every mutating call, WS Origin must match unless the bind is loopback (identity-and-security.md lines 32-81). Flags: `--headless --host 127.0.0.1 --port
   --no-https --auth --storage-dir --reticulum-config-dir --identity-file --generate-identity-file`. Plugins can only hook `announce.received` and
   `rns.link.event`, not messages (`docs/en/plugins.md`). The KB's open question "does the fork keep the send API" is answered: yes.
8. **MeshChatX install weight**: 20 hard requirements including `wasmtime`, `aioquic`, `miniaudio`, `bleak`, `lxst`; `miniaudio` has **no Linux aarch64 wheel** (sdist C build on Pi class
   nodes; PyPI file list), wasmtime has android wheels but Termux's pip likely will not select them (unverified). Its own Termux guide needs `rust`, `binutils`, `build-essential` to build
   `cryptography`. Phones do not need MeshChatX: they can be plain Sideband/Columba peers that connect to the hub (lens 3).
9. **LXMF wire facts** (`LXMF/LXMessage.py`, `LXMF.py`, `LXMRouter.py`): message = dest 16 + source 16 + Ed25519 sig 64 + msgpack `[timestamp f64, title, content, fields(, stamp)]`
   (the README prose lists content before title; the code is title first). Overhead 112 B. Opportunistic single packet: content ≤ 287 B; over a link as one packet ≤ 319 B; above
   that a Reticulum Resource over a link (setup cost matters on LoRa). Over TCP the practical limit is the receiver's `delivery_transfer_max_accepted_size` (library default 1,000 KB; lxmd 1 KB).
   Message id = SHA-256(dest ‖ source ‖ payload), not transmitted. Timestamp is the sender's `time.time()`, signed data, not verified: a node with no RTC sends wrong times.
10. **Delivery meaning**: states OUTBOUND, SENDING, SENT, DELIVERED, REJECTED, CANCELLED, FAILED. `DELIVERED` = recipient proof (opportunistic packet proof, or link/resource completion). A propagated
    send ends at `SENT` (a propagation node holds it). `MAX_DELIVERY_ATTEMPTS = 5`, `DELIVERY_RETRY_WAIT = 10 s`, `PATH_REQUEST_WAIT = 7 s`; after that `FAILED` unless propagation fallback
    is configured. This maps cleanly onto the KB four states: `queued` = no path / OUTBOUND, `sent` = SENDING/SENT, `relayed` = propagated SENT, `delivered` = DELIVERED; our outbox must keep its
    copy and resubmit after FAILED (delivery first), because LXMF gives up.
11. **LXMF 1.2.1 (2026-10-13) is airtime-aware**: diffing the wheel against the 1.2.0 mirror shows `AIRTIME_AWARE_MTU_TIME = 0.5`, a slow-path cache, holding queued messages to a slow destination,
    and receipt timeouts scaled to the measured round trip. Good for LoRa; it also means LXMF will hold messages we hand it, so our "sent" must follow its state callbacks, not our own timers.
12. **Stamps and tickets, what a sender pays**: a recipient announces `stamp_cost` in its delivery announce app_data `[display_name, stamp_cost, supported]`; the sender must find a stamp whose
    SHA-256 over a 768,000 B workblock (3000 HKDF rounds × 256 B) has `cost` leading zero bits: expected 2^cost attempts, each hashing the whole workblock (`LXStamper.py` lines 12, 49-78, 178-200).
    Cost 12 ≈ 4,096 attempts ≈ 3 GB hashed per message (order of 10 s on a Pi-class core, estimate). Propagation stamps cost 16 (min 13), peering 18. A `FIELD_TICKET` (0x0C) grant lets that sender skip
    stamps for 21 days (`TICKET_EXPIRY`). Enforcement is off by default in the library. Our own node should announce no stamp cost (no policy limits); we must pay other peers' costs.
13. **Fields useful to a bridge**: `FIELD_CUSTOM_TYPE/DATA/META` (0xFB-0xFD) are reserved "for bridging/tunneling external protocols"; `FIELD_THREAD` 0x08, `FIELD_REPLY_TO` 0x30, `FIELD_RENDERER` 0x0F
    (plain/micron/markdown/bbcode). Via MeshChatX only attachments and `app_extensions` are writable, so carrying a sealed 111 envelope over LXMF (the L5 `rns-bridge` seam, 287 B budget,
    `5-messaging/README.md:364`) would ride as a small file attachment, which is awkward but possible; Option C could use the custom fields directly.
14. **Discovery**: LXMF peers appear in others' apps only by announce (≈200 B with ratchet, estimate) or when someone imports our address + public key. Announce cap is 2 % of each interface's bandwidth by default
    (`Reticulum.ANNOUNCE_CAP = 2`). Our AutoInterface uses `group_id = 111.network` (`config.tmpl:15`); stock apps default to group `reticulum` (`AutoInterface.DEFAULT_GROUP_ID`), ports 29716/42671,
    so stock LAN users will not hear us unless we join their group or they connect over TCP. Two AutoInterface blocks in one rnsd share default ports (sockets set `SO_REUSEADDR/REUSEPORT`); behaviour unverified.
15. **Identity and A10**: an RNS identity file is 64 raw bytes (32 X25519 private ‖ 32 Ed25519 seed; `Identity.get_private_key`/`load_private_key`); the public key is 64 B. L2 §10's 32 B `transport_key`
    field therefore binds by SHA-256 of the 64 B public key (`2-identity/README.md:366`), and `transport_sig` is a plain Ed25519 signature 111d can make from the seed. Cleanest flow: the *child* creates
    its identity (`meshchatx --generate-identity-file`, or lxmd on first run), 111d reads the file and signs the attestation. 111d never writes or parses anything Reticulum-specific except that 64-byte file.
    One delivery identity per `LXMRouter`, so one LXMF address per bridge identity (matches "one bridge identity per foreign network"); MeshChatX supports several identities if per-user addresses are ever wanted.
16. **No channels**: confirmed. Closest patterns are (a) a distribution-group endpoint that fans out (LXMF-Tools `lxmf_distribution_group_*`, MIT, beta), (b) Reticulum Relay Chat, a separate IRC-like protocol over
    Reticulum links (rrcd, MIT; MeshChatX has an RRC client), (c) `FIELD_GROUP` with no published meaning. A relayed message re-emitted into LXMF must come from our bridge identity (a source cannot be spoofed);
    put the foreign sender in the text prefix, as the owner's "labelled honestly" rule requires.
17. **Reimplementation is not a safe v1 path.** Upstream's chapter (`docs/source/brandolinis.rst` lines 127-142, 153, 195, 203) says the protocol is public domain and "any human" may implement it from documentation,
    but a machine-generated reproduction of the source is a derivative work. This repo is built with LLM agents, so a Rust LXMF stack written by them is exactly the contested case; third-party stacks
    (Leviculum, LXMF-rs, Reticulum-rs 0.1.0 from 2025-10-14) are not upstream and break D17 #12.
18. **Test hygiene**: the far-end LXMF peer in netns tests must be an adopted program (MeshChatX or an LXMF-Tools echo bot), never a test script of ours that `import LXMF`, or the "no import in our tree" evidence line (`LICENSING.md:30`) breaks.

## 4. Recommendation

**Posture (all options).** Render `config.tmpl` v2 (bump `SCHEMA_VERSION`, `config.rs:11`; the takeover seam at `mod.rs:148-153` is built for this):
`share_instance = yes`, `instance_name = 111`, `rpc_key = <32 random bytes hex, from the vault>`, `enable_transport = no` (an endpoint needs no transport; a phone connected by TCP reaches our LXMF
address directly). Keep the `111.network` AutoInterface. Optional, owner-gated: one `BackboneInterface` (Linux and Android; wire-compatible with TCP client/server) on the LAN address, port 4242, with `network_name` +
`passphrase` (IFAC) so stock Sideband/Columba on phones can connect; optional second AutoInterface in group `reticulum` for stock LAN peers. Start order: rnsd, wait for the abstract socket, then children;
restart children on rnsd restart; use `rnstatus --attach/--reload` instead of killing rnsd for interface changes. Client dirs: one empty-interface config dir per child with the same `share_instance`, `instance_name`, `rpc_key`.

**The fork.**

- **A. MeshChatX child (pick).** 111d runs `meshchatx --headless --host 127.0.0.1 --port <11xxx> --no-https --storage-dir … --reticulum-config-dir … --identity-file …` from a hash-locked venv (`reticulum-meshchatx==4.10.7`,
  rns/lxmf pinned from PyPI), sets a vault-held password through `/api/v1/auth/setup` with `--auth`, logs in with a cookie jar and CSRF token, and runs a Rust adapter (a `Link`-sibling "carrier", ~300 lines: poll/WS in, `send` out, state mapping to the four states).
  Dedupe/loop guard by message hash in 111d (no on-wire tags needed). The Reticulum panel is the same process behind the L4 panel proxy (needs WS proxy support = P7). Trade-offs: sunset, internal API, no verify flag
  (label foreign senders "Reticulum, sender unverified"), heavy install, Termux unproven.
- **B. lxmd hook.** Tiny, official, licensed like Reticulum, but inbound-only with no sender, no names, no verify flag. Use only as an opt-in propagation node (`enable_node = yes`, 500 MB default) if the owner wants the hub to store
  messages for Reticulum neighbours; MeshChatX can also run a propagation node itself, so B adds nothing to the pick.
- **C. Sidecar importing RNS/LXMF (rule change, not default).** One Python file in its own hash-locked venv, JSON Lines over a loopback socket to 111d. Gains: stable public API (the README examples), `signature_validated` and
  `unverified_reason` per message, state callbacks, `FIELD_CUSTOM_*` and title, tickets/stamps control, ~30 MB lighter, pure-Python on Termux, no web UI to secure, no sunset. License consequences, spelled out:
  (1) D17 #12 and `LICENSING.md` §1/§7 ("no Reticulum code incorporated, linked or derived in our code"; evidence line "no crate depends on Reticulum") must be amended to allow one of our source files that imports the
  user-installed packages; (2) at run time the sidecar plus Reticulum form one program, so the Reticulum License's no-harm and no-AI-training conditions govern that program; those field-of-use restrictions are not
  GPL-compatible (§7), so the sidecar must not be linked into or combined with `111d`, must stay a separate process, and should carry a permissive licence (0BSD/MIT) or GPL-3.0-or-later with an explicit note that the
  user installs `rns`/`lxmf` themselves (needs a legal read; the existing precedent, the presence reader, is GPL only because meshtastic-python is GPL); (3) the AI-training clause means no Reticulum code, docs or data
  into any training or fine-tuning set, which §8 already records; (4) upstream's "fakes" warning targets reimplementations, not importers of the unmodified packages. I recommend asking the owner for C **if no MeshChatX
  successor is announced by v5.0.0**, because the carrier trait makes the swap one module.
- **D. Anything else (reject).** Own Rust LXMF, Leviculum/LXMF-rs/Reticulum-rs (not upstream, legal exposure per finding 17), LXMFy cogs (command bots), LXMF-Tools MQTT bridge (custom fields, beta), NomadNet, rnsh.

**Pick: A, behind a carrier trait, with C held ready as the owner's contingency.** Reasoning: A keeps every rule intact and delivers the owner's "full UI plus text in our Messages view" in one adopted
program; its deadline falls before Layer 6 hardware exists, so the decision on C should be made before the carrier is built, not after.

**What the Reticulum category does in v1.** Inbound: every LXMF message appears under "Reticulum", thread per peer address, name from the last announce, state "sender unverified" (A/B) or verified (C). Outbound:
text to a chosen peer in native LXMF (opportunistic when content ≤ 287 B, otherwise MeshChatX picks link/propagation), stamps paid automatically. Relay (opt-in, loop guard): a message from another network is re-sent as
`[Meshtastic !a1b2c3d4] text` from the Reticulum bridge identity; dedupe by foreign message id + content hash.

## 5. Open questions for the owner

1. Accept A knowing it ends on 2027-01-01, or lift D17 #12 for a sidecar (C) now? My default: A now, decide C when v5.0.0 lands without a successor.
2. What is the "send to Reticulum" target without channels: a chosen LXMF peer (a contact's 32-hex address), a distribution-group hub, or an RRC room later? For "same message in several networks" I assume per-network recipient lists.
3. May the node join the public `reticulum` AutoInterface group and/or open a LAN `BackboneInterface` with IFAC for stock apps? Both widen exposure; IFAC passphrase handling needs a UX.
4. Should the hub offer itself as an LXMF propagation node (opt-in; storage, stamp cost 16/13 implications)?
5. Display name and announce interval for the bridge identity (LoRa 2 % announce cap; stock apps only learn our key by announce).
6. Is "sender unverified" acceptable in v1 labels, or must verified/unverified be shown (forces C)?
7. Clock: LXMF stamps wall-clock into signed messages; nodes without an RTC need a time source or a visible caveat.
8. Relay labelling: foreign sender in a text prefix from our bridge identity, source never spoofed. OK?

## 6. Sources

- https://pypi.org/pypi/rns/json · https://pypi.org/pypi/lxmf/json · https://pypi.org/pypi/nomadnet/json · https://pypi.org/pypi/reticulum-meshchatx/json · https://pypi.org/pypi/lxmfy/json · https://pypi.org/pypi/wasmtime/json · https://pypi.org/pypi/miniaudio/json
- https://raw.githubusercontent.com/markqvist/Reticulum/master/LICENSE · …/Changelog.md · …/RNS/Reticulum.py · …/RNS/Identity.py · …/RNS/Interfaces/LocalInterface.py · …/RNS/Interfaces/AutoInterface.py · …/RNS/Utilities/rnstatus.py
- https://raw.githubusercontent.com/markqvist/Reticulum/master/docs/source/{using,interfaces,software,gettingstartedfast,brandolinis}.rst · https://reticulum.network/manual/
- https://raw.githubusercontent.com/markqvist/LXMF/master/{LICENSE,README.md,MIRROR.md,LXMF/LXMessage.py,LXMF/LXMF.py,LXMF/LXMRouter.py,LXMF/LXStamper.py,LXMF/Utilities/lxmd.py}; LXMF 1.2.1 wheel from https://files.pythonhosted.org/packages/90/90/89a1ba2135cd9bf9bb98f41ed3b9379b8079a2c2c6473e246029ff1eee58/lxmf-1.2.1-py3-none-any.whl
- https://github.com/Quad4-Software/MeshChatX · https://raw.githubusercontent.com/Quad4-Software/MeshChatX/master/{LICENSE,README.md,CHANGELOG.md,docs/en/{installation,identity-and-security,architecture,messaging,plugins,rns-link-api,platform-guides/android-termux}.md,meshchatx/meshchat.py,meshchatx/src/backend/lxmf_utils.py,meshchatx/src/backend/http/routes/{lxmf,auth}.py}
- https://raw.githubusercontent.com/liamcottle/reticulum-meshchat/master/{LICENSE,package.json,docs/meshchat_on_android_with_termux.md}
- https://raw.githubusercontent.com/Quad4-Software/LXMFy/master/{LICENSE,README.md,docs/creating-bots.md} · https://raw.githubusercontent.com/SebastianObi/LXMF-Tools/main/{LICENSE,README.md}
- https://raw.githubusercontent.com/markqvist/NomadNet/master/LICENSE · https://raw.githubusercontent.com/markqvist/Sideband/master/LICENSE · https://raw.githubusercontent.com/torlando-tech/columba/main/LICENSE.md · https://github.com/torlando-tech/columba
- https://codeberg.org/Lew_Palm/Leviculum · https://crates.io/api/v1/crates/lxmf-wire · https://github.com/FreeTAKTeam/LXMF-rs · https://crates.io/api/v1/crates/reticulum
- https://github.com/kc1awv/rrcd · https://raw.githubusercontent.com/kc1awv/rrcd/main/LICENSE · https://github.com/RFnexus/Reticulated · https://zff.dev/jahway603/send-lxmf
- Repo: `LICENSING.md:24-31,59,184-185` · `plan/cross-layer/devices.md` §7 item 12 · `1-bootstrap/crates/node111/assets/rns/config.tmpl:8-15` · `…/src/daemon/transport/{mod.rs:102-113,148-186,config.rs:11-22}` · `2-identity/README.md:223-236,366` · `5-messaging/README.md:364` · `5-messaging/crates/mesh111/src/link/mod.rs:255` · `0-knowledge/{projects/{lxmf,meshchatx,reticulum}.md,concepts/store-and-forward-states.md}`
