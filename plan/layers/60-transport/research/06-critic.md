# Layer 6 research: completeness critic (06)

Date 2026-10-19. I read reports 00–05 in full and checked them against D23, D17, D19 c, D20, D22, README §6, Layer 5 §8/§19/§21 and devices.md §0/§3/§7–§9. I ran six targeted checks of my own (listed under "Settled here"). The full scout prompts are in the structured return.

## Verdict

The sweep is strong on code seams (00), Meshtastic firmware behaviour (01), bridge semantics (04) and transport sequencing (05). It is enough to brief the store, categories, fan-out, relay and loop-guard forks today. It is **not** enough for the LXMF fork. The owner is asked to choose between MeshChatX (A) and a D17 #12 rule change (C) before anything is built, yet the pick was never run. The pick also conflicts with the device floor: MeshChatX needs Python ≥ 3.11, while our venvs use the distro `python3`; the lead box has 3.10.12. Close that gap first. Four major gaps should also land before the brief, because each one changes a pick or touches a hard rule.

## Gaps

| # | Gap | Severity | Why it matters |
|---|---|---|---|
| G1 | The LXMF endpoint was never run, and its fit on the device floor is unknown | **blocking** | PyPI `reticulum-meshchatx` 4.10.7 has `requires_python >= 3.11` and 21 deps (miniaudio, wasmtime, aioquic, landlockpy, seccompy…). Venvs are built from the distro `python3` (`10-bootstrap/README.md:53,146`), so Ubuntu 22.04 and Debian 11 cannot install it. Attaching to the shared instance, the send/receive JSON, field-16 pass-through and outbound egress (telemetry, update checks) were all read from source, never executed. A vs C cannot be judged without these facts. |
| G2 | Meshtastic: own client picked over adoptable ones; MeshMonitor not weighed | major | Lens 1 picks its own ~300-line client (A1), though it measured the size cost as negligible itself. The official `meshtastic` crate meets lens 1's own constraints (see below). MeshMonitor, already in the KB, has a REST `POST /messages` for channel and DM text (auto-split above 200 B) and `GET /messages`, yet appears in no report. |
| G3 | No proven hardware-less harness for Meshtastic | major | D23 #4 wants stubs first. `meshtasticd -s` has no prebuilt binary (lens 5). Meshtasticator's interactive simulator runs N native nodes on TCP 4403+ and forwards packets between them, but no scout found it. Nobody knows whether a second TCP client (meshmud) can attach alongside the simulator's own client. The table in lens 5 §4.1 overstates what is provable today. |
| G4 | The phone-apps tier versus the device floor and offline-first | major | Lens 3 moves "phone apps through Termux" onto a glibc Debian-class hub (SimpleX Haskell binaries, a Postfix/Dovecot chatmail stack), against hard rule 5. It missed **Madmail** (single-binary chatmail relay, IP plus self-signed mode; GitHub metadata says AGPL-3.0, Rust, pushed 2026-10-17). `deltachat-rpc-server` speaks stdio JSON-RPC, but the supervisor sets `stdin = null` (`N/supervise.rs:333`). No report says that the strongest offline phone-app paths are Columba/Sideband over the LXMF bridge and the Meshtastic app over BLE, i.e. the paths of networks 1 and 2. |
| G5 | README §6's "ordered list of preferred networks … join in sequence … fall back" is unaddressed | major | It is the layer's own spec text (`README.md:85`), and no report touches it. The brief must either scope it or defer it explicitly. |
| G6 | Rule-1 provenance of the RNode probe bytes and the stub | major | Lens 5 takes the DETECT bytes from `RNodeInterface.py:490-493` and models the stub on what RNodeInterface expects. That is derivation from Reticulum-licensed source, by LLM agents, which is exactly the case `brandolinis.rst` (lens 2 f17) warns about. Re-source from GPL-3.0 `RNode_Firmware`, or keep adopted `rnodeconf --info` (devices.md §3). |
| G7 | Meshtastic frame-budget arithmetic | minor | L5 §8 pins 233/221 B for `PRIVATE_APP`. By the firmware check (255−16, minus 12 for PKI) and the encoding of `Data` (portnum 256 is a 2-byte varint, plus a bitfield), the budget is about 231/219 B. Text comes out at about 232/220 B. The lead can settle this with a prost encode. |
| G8 | No adopt-heavy alternative to the bridge core was weighed (a Matrix or XMPP hub as the bus) | minor | "Adopt, don't reinvent" means the brief should show it was considered. D23's "text in our Messages view" probably rules it out. |
| G9 | The protobufs licence version | minor | The LICENSE file holds the GPL-3.0 text with no version notice. Under GPLv3 §14, lens 1's "effectively GPL-3.0-only" claim may not hold. It needs a legal read; it is not a blocker. |

## Conflicts to resolve before the brief

1. **Adapter channel.** Lenses 0 and 4 put LXMF and phone-app adapters in spool-dir sidecars. Lens 2 has 111d act as a REST/WS client of MeshChatX, with no sidecar of ours. Lens 3 relies on a SimpleX WebSocket and on stdio JSON-RPC, which `supervise.rs:333` blocks.
2. **LXMF relay marker.** Lens 4 puts it in `FIELD_CUSTOM_TYPE/DATA` with no text in the body. Lens 2 found that MeshChatX's send API takes no arbitrary field ids (attachments and field 16 only), and proposes a visible prefix. Lens 2's hash dedupe works only inside one node, while lens 4's guard has to work across 111 nodes. A marker with no body text would also hide the original sender from stock Sideband users, which breaks the honest-label rule.
3. **`enable_transport`.** Lens 2 keeps it `no`. D17 #5 decided `yes` on Cat 3/4, and devices.md §2.2c's phone relay depends on that.
4. **Meshtastic text ceiling.** Lens 1 advertises 200 B. Lens 4's preview refuses at 233 B. L5 pins 233/221 B. The firmware gives about 232/220 B (see G7).
5. **Adopt vs own.** Lens 1 picks its own client over the crate (G2). Lens 5 picks its own byte probes over `rnodeconf --info`, against devices.md §3 (G6).
6. **MeshCore ID before the lock.** D20 requires registration before Layer 6 locks. Lens 5 shows upstream grants an ID only after a working demo, so this goes to the owner.
7. **Writing node config.** Lens 1 says never write it. Devices.md §3 (D17) allows a typed `meshtastic.set` allowlist.
8. **PSK handling.** Lens 4 keys conversations by `psk-fingerprint`. Lens 1 says PSKs are never persisted. A keyed hash would satisfy both.
9. **MQTT-uplinked channels.** Lens 5 says never send on them by default. Lens 1 says warn before the first send.
10. **Sequencing.** Lens 5 wants bridge first (A2). Devices.md §8 puts the device core first. This is a real fork for the owner.
11. **meshmud releases.** Lens 5 says none, lens 1 says v1.0.0. **Settled: lens 1 is right.**
12. **MeshCore 1.18 loop-detect default.** The release blog and `cli_commands.md` disagree, and lens 5 left it open. It does not matter in v1.

## Settled here (critic checks)

- **meshmud has a release.** `gh api repos/Rikonardo/meshmud/releases` shows v1.0.0 (2026-03-20) with linux amd64/arm64 tar.gz and deb assets.
- **meshmud never sends a heartbeat.** Its only originated `ToRadio` is one `want_config` on uplink connect (`internal/common/uplink.go:95-106`, random nonce, `protobuf/toradio.go`). That supports lens 1's 15-minute idle-lapse defect on the meshmud side. **Route it to the Layer 5 closure now**: Nearby can freeze while no panel is open. G3's harness can confirm the firmware side on meshtasticd, which runs the same PhoneAPI/ServerAPI code.
- **The `meshtastic` crate fits.** It sends `Heartbeat::default()` (nonce 0, `src/connections/handlers.rs:226`) every 5 minutes, takes a config id we choose (`stream_api.rs:541-546`), and `connect<S>` accepts any stream, so 127.0.0.1:11230 works. Lens 1's constraints do not exclude A2.
- **The MeshChatX sunset is confirmed.** Its README says "Development and releases end on the 1st of January 2027", with v5.0.0 the last release. PyPI shows `requires_python >=3.11` and lists its dependencies.
- **MeshMonitor's API.** Its OpenAPI defines `POST /messages {text, channel | toNodeId, replyId}`, which returns 201 or 202 (split); 413 above three parts; REST only, with no stream.
- **Meshtasticator.** Its README states CC BY 4.0, which is acceptable for a test-only tool. Each node gets its own TCP port starting at 4403, and the script forwards packets between nodes by simulated range.

## Honesty notes for the brief

- **Phone apps.** Lens 3 says honestly that P2 as worded (the agent installs and drives apps on the phone) is not feasible without user-attended ADB, and that Briar is Tor-only. The brief must lead with that, then present the real offline phone paths (G4). It should not present SimpleX or Delta Chat as "through Termux".
- **Hardware.** Lenses 1 and 5 list hardware gates well. Several claims still read as verified when they are not: lens 5's "LXMF bridge: none blocked" (nothing was run), "meshtasticd -s" (no binary obtained), and every phone-app claim (no runs). The brief should mark each fork's evidence as *run*, *source-read* or *doc-only*.
