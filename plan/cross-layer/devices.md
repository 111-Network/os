# Device-connection core — plan brief (Layer 6 opener)

Status: ✍️ brief with owner decisions (D17, 2026-10-14) · task P1 (`os/40-interface/TASKS.md`) · build at Layer 6 kickoff.
Reads with: `os/README.md` (Cat 1–5, Layers 6–7) · D9/D12 (phone path, rnsd child, meshtasticd detection-only,
phone multicast gap) · D10/D15 (agent tools, registry, prompts) · `40-interface/README.md` §6.3, §7.1, §8 ·
`40-interface/api/openapi.yaml` (`DeviceRecord`).

Owner intent (verbatim): "its super important to have a section for connected devices as well where user can also
name them, the details of the connected devices. e.g. users should be able to make an old phones wifi to connect
with others. … users can connect a custom LoRa or an old phone or anything (maybe later even baofeng radios!!) to be
able to communicate in this network!"
Principle (verbatim): "the agent looks for whats connected or paired and resolves errors and solves issues etc to make
the system work. this way, we dont need a specific hardcoded instructions for each device type! everything agentic
and based on what it is."

## 0. Findings that change the plan

1. **USB IDs name the bridge chip, not the board or its firmware.** `10c4:ea60` is any CP2102 cable; `1a86:*` is every
   WCH CH340/CH341 part — including Baofeng programming cables and the CH341 USB-to-SPI LoRa sticks that meshtasticd
   drives natively (MeshStick) — so today's `lora-table.toml` will label those "LILYGO T-Beam". `303a:*` is any
   ESP32-S3 board. The same Heltec V3 can run Meshtastic, RNode, or MeshCore firmware with identical USB IDs.
   Firmware is only knowable by talking to the port.
2. **meshtasticd does not drive a USB Meshtastic board.** It is Meshtastic for Linux hosts whose radio is an SPI HAT or
   a CH341 USB-SPI adapter (root/GPIO; https://meshtastic.org/docs/hardware/devices/linux-native-hardware/).
   A Heltec flashed with Meshtastic runs its own firmware; the host speaks the client API over serial/BLE/TCP
   (https://meshtastic.org/docs/software/python/cli/usage/). Layer 4's `radio` service ("live = TCP 4403") and R4's
   `needs_setup` ("LoRa board without meshtasticd") never go live for the most common board, and "Set up with the
   agent" would steer users to install a daemon that cannot drive it.
3. **Opening a serial port can reset the board.** ESP32 dev boards wire DTR/RTS to EN/IO0; some open sequences hold
   the chip in reset (https://docs.espressif.com/projects/esptool/en/latest/esp32/advanced-topics/boot-mode-selection.html).
   Active identification is a side-effecting act: consented, port-locked, rate-limited.
4. **Our rnsd cannot serve anyone else yet.** `assets/rns/config.tmpl` sets `share_instance = no` (local LXMF apps
   cannot attach) and `enable_transport = no` (the node will not route for a phone). Both defaults are documented in
   https://reticulum.network/manual/using.html. Phone-as-relay needs a config posture change, not just a device.
5. **Termux sees no serial, BLE, multicast, or hotspot.** USB is reachable only as a raw fd from `termux-usb` after an
   on-screen approval per device; pyserial cannot open it, so rnsd's RNodeInterface cannot use a phone-attached RNode
   (https://github.com/nohajc/termux-adb, https://github.com/schuhumi/Termux_CDC_ACM). BLE, `MulticastLock`
   (https://developer.android.com/reference/android/net/wifi/WifiManager.MulticastLock) and hotspot control are
   Android-app APIs. Every phone-side device capability therefore hinges on one Android app (ours or adopted).
6. **Transmit legality is a per-node human fact.** LoRa needs the ISM region plan before TX (Meshtastic ships region
   unset; `rnodeconf` takes raw Hz with no region guard). Amateur bands forbid "messages encoded for the purpose of
   obscuring their meaning" (US 47 CFR §97.113(a)(4), https://www.ecfr.gov/current/title-47/chapter-I/subchapter-D/part-97/subpart-B/section-97.113),
   which is exactly our sealed envelope (L2 C1). Baofeng-class radios are not FRS/GMRS-certified in the US
   (FCC advisory DA 18-980, https://docs.fcc.gov/public/attachments/DA-18-980A1.pdf). The agent may suggest; a human confirms.
7. **The best phone app candidate has an adoptable license; the reference app does not.** Sideband is CC BY-NC-SA 4.0
   (actively released — 2.2.0 on 2026-10-01; corrected 2026-10-14, the earlier "no longer updated" note was stale;
   https://github.com/markqvist/Sideband) — it fails the KB's OSI gate on license alone.
   Columba is MPL-2.0, native Android, Reticulum/LXMF over BLE, TCP and RNode (https://github.com/torlando-tech/columba).

## 1. Scope & definitions

- **Device** — hardware the user owns, attached to or paired with *this* node: a radio on USB/SPI/BLE, a phone or
  laptop paired as companion, an audio-cabled handheld, another 111 node they own. Shown under Devices.
- **Service** — software the node runs or detects (`agent`, `model`, `transport`, `radio`, `kb`, `gateway`,
  `codeagent`; `ServiceId` enum). A device is *driven* by a stack; the registry links them via `service_id`.
- **Nearby node** — someone else's node heard over a transport. Never a device; lives under Nearby (Layer 7).
- Registry enums the UI already consumes (`openapi.yaml`, quoted):
  `DeviceKind: [radio, bluetooth, companion, node, analog, unknown]` ·
  `DeviceState: [connected, disconnected, needs_setup, error]` ·
  `DeviceTransport: [usb-serial, bluetooth, wifi, lan, audio, unknown]` · `actions: [rename, forget, agent]` ·
  `reason_code` examples: `present, unplugged, no_driver_service, service_error, session_active, session_idle`.
- **The core** = a `devices/` module in `111d` (detect, identify, bind, monitor) + the rnsd config renderer (Layer 1's
  `schema-version` seam) + a typed agent tool surface + a narrow applier. The agent decides *what* a thing is and
  *what* to do; `111d` executes closed, schema-checked actions. No per-device scripts — device knowledge lives in KB
  entries the agent reads.

## 2. Device classes & paths

### 2.1 USB/serial LoRa boards (Heltec, LILYGO, RAK, Seeed …)

**D19 (c) amendment, 2026-10-14:** one serial port has one owner — each board carries `owner = 111d | rnsd` in
`state/interfaces.toml`. A **111-owned** board runs the **MeshCore KISS modem** firmware (MIT; documented raw-modem
protocol: 255 B frames, CSMA, `SetRadio`, airtime report) and `111d` emits MeshCore `GRP_DATA` frames (163 B payload)
that stock MeshCore repeaters flood blind — the Cat-1 relay is therefore the stock MeshCore repeater, and 111 writes no
relay firmware. An **rnsd-owned** board runs RNode as before. Opt-in "shared" mode: 111d owns an RNode in HOST mode and
rnsd attaches by loopback `TCPClientInterface kiss_framing=True` (111 frames are then RNode-framed and invisible to
foreign repeaters). Full analysis: `layers/50-messaging/research/07-gap-1.md`; the fabric side: `layers/50-messaging/options.md` §2.3.

Detection today: Layer 1's probe + R4 read sysfs VID:PID + product string against `lora-table.toml` (6 rows) and a
udev rule sets `ID_MM_DEVICE_IGNORE`. That yields "likely a LoRa board", not "which firmware" (finding 1).

| Firmware on board | Host side | Driving stack | "Set up" means |
|---|---|---|---|
| **RNode** (GPL-3.0; Heltec V3/V4, T-Beam, T3S3, T-Echo, RAK4631 … https://github.com/markqvist/RNode_Firmware) | dumb LoRa modem; host owns crypto + routing | rnsd `RNodeInterface` (`port` = serial path, `tcp://` for Wi-Fi RNodes, `ble://` once paired; `frequency`, `bandwidth`, `txpower`, `spreadingfactor`, `codingrate`; https://reticulum.network/manual/interfaces.html) | write one interface stanza with region-legal params; restart rnsd; verify interface up |
| **Meshtastic** (GPL-3.0) | full mesh node; host is a client over serial/BLE/TCP (protobuf API) | meshtastic-python (CLI/lib), not meshtasticd | set region/preset/role via the client API; read nodes heard; bridge later |
| **MeshCore companion** (MIT) | companion radio; one link per build (serial or BLE or Wi-Fi; https://github.com/meshcore-dev/meshcore-cli/wiki) | `meshcore` py lib / `meshcore-cli` (https://github.com/meshcore-dev/meshcore_py) | read contacts/adverts; bridge later |
| **MeshCore KISS modem** (MIT; the *111 modem* profile, D19 c) | raw LoRa modem; host builds MeshCore `GRP_DATA` frames | `111d` (`mesh111` `kiss-modem` link) | CI-built, manifest-pinned image (no upstream release tag for the kiss-modem env); region set once (decision 10); stock MeshCore repeaters relay |
| **blank / unknown** | — | none | offer a firmware (decision 1); flashing is destructive and human-gated |

- `rnodeconf` (ships inside the `rns` wheel we already pin) does `--autoinstall`, `--info`, `--bluetooth-pair`,
  `--tnc`, EEPROM bootstrap and device signing; it fetches firmware online by default (`--fw-url`, `--nocheck`,
  `--use-extracted` exist; https://reticulum.network/manual/using.html). Offline nodes need firmware images pinned in
  the signed channel manifest, like the APKs.
- Experimental: Reticulum *over* an unmodified Meshtastic radio (https://github.com/landandair/RNS_Over_Meshtastic),
  ~500 B/s on ShortFast per its author — a watch item, not a path.
- **SPI HAT / CH341 radios** (MeshAdv-Pi, MeshStick): the only case meshtasticd drives (TCP API 4403, web UI on the
  `Webserver` port, `/etc/meshtasticd/config.yaml` + `config.d/`; https://meshtastic.org/docs/linux/usage/). Not USB
  serial — detect via `/dev/spidev*` + device-tree overlays or CH341 `1a86:5512`. This is where the existing `radio`
  service probe is correct.

### 2.2 Old phones — three roles (one device, different jobs)

- **(a) A node of its own.** Exists: Layer 1 `--phone` over USB-C (Termux + run-as + Termux:Boot; D9 E1). Limits: no
  USB serial, no BLE, no multicast receive (D12) → sits `isolated` without a hub. Fix path: a TCP uplink to a hub
  (works today with unicast) or an Android helper app (decision 3).
- **(b) A companion / light terminal of this node.** Exists as a paired dashboard session (`companion`, browser,
  pair code + cookie; D15). Not a mesh participant: it controls *this* node and messages through it once Layer 5
  ships. To become a real Cat 2A terminal with its own identity, it needs a Reticulum/LXMF client (Columba, or the
  Termux node of role a) connected to this node — which requires `share_instance`/TCP server + transport on (finding 4).
- **(c) A relay / Wi-Fi island — "make an old phone's Wi-Fi connect others".** Patterns, in order of readiness:
  1. **Phone joins a LAN; node runs a TCP server.** rnsd `TCPServerInterface`/`BackboneInterface` on the LAN (opt-in
     like LAN sharing) + `enable_transport = yes` on hubs/bridges; phone app dials `111.local` (mDNS already announced
     when LAN sharing is on) or a QR. Unicast — no MulticastLock needed.
  2. **Phone is the hotspot.** User-enabled tethering today (Termux cannot toggle it); `startLocalOnlyHotspot`
     (https://developer.android.com/reference/android/net/wifi/WifiManager) gives an app a random SSID/passphrase
     to show as a QR. Other phones and laptops join its Wi-Fi; AutoInterface or TCP runs over it. Unverified:
     client-to-client multicast on OEM hotspots (gate G4).
  3. **AutoInterface on shared Wi-Fi** (`group_id = 111.network`, IPv6 link-local multicast; UDP 29716/42671 per the
     manual). Zero-config on Linux; on phones blocked by the MulticastLock gap until an app holds the lock.
  4. **BLE phone-to-phone.** Columba ships `ble-reticulum` (BLEInterface/BLEPeerInterface, Android driver;
     https://codeberg.org/Lew_Palm/leviculum/issues/83). A Linux/BlueZ driver is not confirmed. Experimental.

### 2.3 Bluetooth devices

- **Host adapters** (`/sys/class/bluetooth/hci*`) — detected today as `bluetooth`; they are capabilities, not peers.
- **BLE radios**: RNode over BLE (`ble://`, pair first; `rnodeconf --bluetooth-pair`), Meshtastic over BLE
  (`meshtastic --ble-scan` / `--ble`; first connect asks the PIN; on Linux pair via `bluetoothctl` first; one BLE
  client at a time — owning it locks out the user's phone app), MeshCore BLE companions.
- Linux stack: BlueZ over D-Bus. `111d` needs a pairing agent (PIN entry from the dashboard) and D-Bus policy for
  `node111`; Rust binding `bluer` (official BlueZ crate) vs shelling to `bluetoothctl`. Scanning is active radio use:
  user-triggered, time-boxed, never background.
- Phones: BLE needs an Android app (finding 5).

### 2.4 Analog handhelds (Baofeng-class)

- Path: radio ↔ audio/PTT cable (AIOC — USB sound card + serial PTT, Baofeng K-plug, stable as of 2025;
  https://github.com/skuep/AIOC — or Digirig) ↔ Dire Wolf soundmodem (GPL-2.0, AX.25 1200 AFSK, KISS over TCP 8001;
  https://github.com/wb2osz/direwolf) ↔ rnsd `TCPClientInterface` with `kiss_framing = True`, or `KISSInterface` /
  `AX25KISSInterface` with `id_callsign` (manual, interfaces page).
- Maturity: each piece is mature for APRS/packet; the chain into Reticulum is documented but rare. Throughput ≈ tens of
  bytes/s effective at 1200 baud; 9600 needs a radio with a flat-audio data port (UV-5R has none).
- Legal: amateur use needs a license and plaintext (finding 6) — our E2E-sealed messaging cannot lawfully ride it in
  the US; only plaintext classes (beacons, presence, public bulletins) could. Licence-free bands do not admit
  Baofeng-class radios in most jurisdictions.
- M17 (open digital voice/data, https://m17project.org) needs M17-capable radios/firmware (OpenRTX, Module17), not a
  cable on a stock Baofeng — watch list.
- Detection: USB audio class + CDC serial on one device, or a known cable ID → `analog`, `transport: audio`, state
  `needs_setup`, reason "licensed, plaintext-only path — experimental".

### 2.5 Other 111 nodes you own

`node` kind: a mutual, explicit link (pair-code or QR exchanged on both dashboards) producing a signed link record.
Needs Layer 2's attestation format (D13 F1) and Layer 7 presence; there is no "owner identity" today. Out of the
opener; the enum slot stays.

## 3. What the core must do

1. **Detect (passive, continuous).** udev/netlink uevents (fallback: 5 s poll) over `/dev/serial/by-id`,
   `/sys/bus/usb/devices`, `/sys/class/sound` (USB audio), `/dev/spidev*` + device-tree, `/sys/class/bluetooth`;
   BlueZ paired-device list; companion sessions. Stable ids per §7.1. Zero writes to any device.
2. **Identify.** Table match → `likely`; agent + KB → hypothesis; **active probe** (consented once per device by a
   human click) → `exact`: lock the port, open with DTR/RTS held safe, try in order RNode detect (`rnodeconf --info`),
   Meshtastic client handshake (`meshtastic --info`), MeshCore companion query, then release. Adopted CLIs run from
   the existing rns venv (meshtastic/meshcore wheels added to the hash-locked closure); timeouts; ≤ 1 probe/30 s.

   **Layer 5 amendment (D20, 2026-10-14) — the identify seam.** Layer 5 builds the **Meshtastic step** of this sequence now
   (`os/50-messaging/README.md` §14): `POST /devices/{id}/identify` with the consent flag (decision 8) → the daemon's **port-owner
   table** (`HashMap<port, Owner>`; a held port answers `409 port_busy`) → `meshmud` started on the port with the identify
   profile → the presence reader's `my_node_info` through the TCP downlink → `DeviceRecord.firmware` set to
   `{family: meshtastic, version, source: probe}`, `driver.stack = meshtastic`, `service_id = meshtastic`, and the **binding
   persisted in `state/devices.json`**. The identify child becomes the service child in place (a profile swap, never a second
   open), so the DTR/RTS reset risk is tied to this one consented step. For this step the handshake is `meshmud` plus the
   presence reader (its own hash-locked venv), not `meshtastic --info` from the rns venv. The RNode (`rnodeconf --info`) and
   MeshCore steps stay named seams; on phones `can_run_here = false` for USB boards, and a Wi-Fi board is bound by address. **Layer 6 completes the full
   sequence** (RNode, Meshtastic, MeshCore; the port-owner table is reused) and **creates `state/interfaces.toml`, inheriting the
   Meshtastic binding from `devices.json`.**
3. **Authorize / pair.** BLE pairing via BlueZ agent with PIN typed in the dashboard; companions via the D15 pair code;
   phone USB permission is on-device (Android dialog) — surfaced as `needs_permission`.
4. **Drive.** Bind device → stack: RNode → rnsd interface stanza; Meshtastic/MeshCore → a supervised client child
   (`111d` child pattern, like rnsd); meshtasticd only for SPI/CH341. One owner per port/BLE link, recorded.
5. **Bridge.** Hand live interfaces to Layer 6 transports: rnsd is the fabric; Meshtastic/MeshCore stay foreign meshes
   bridged by thin translators after Layer 5 fixes the envelope (README: "translate only the minimal common format").
6. **Monitor.** Feed `DeviceState`: present + bound + interface up → `connected`; present + unbound → `needs_setup`;
   bound stack in error / port busy / probe failed → `error`; gone → `disconnected`. 2-tick debounce (as R4).

**Agent tools** (Layer 3 tool set grows; still shell-free, fixed, schema-validated — D10):

| Tool | Kind | Shape | Gate |
|---|---|---|---|
| `devices.read` | read | `{id?}` → `DeviceRecord[]` + raw facts ≤ 4 KB each | none |
| `transport.read` | read | → rnsd interfaces `{name, type, up, rx, tx, peers}` (from the shared instance / `rnstatus`) | none |
| `device.identify` | active probe | `{id}` → `{family, version, region?, raw}` | per-device human consent flag; port lock; rate limit |
| `device.propose` | write intent | `{id, actions: Action[], why}` → proposal `kind: device` | human accepts in UI; `111d` validates, applies, verifies, auto-reverts |
| `service.restart` | control | `{service_id}` — `111d` children only | rate-limited 1/5 min/service, logged as an agent event (decision 7) |
| `device.flash` | destructive | `{id, image}` — manifest-pinned image id only | never autonomous: proposal + 2-step confirm naming the device |

`Action` is a closed vocabulary: `rns.interface.upsert {device_id, type, params}` · `rns.interface.remove {name}` ·
`rns.posture.set {share_instance, enable_transport}` · `radio.region.set {region}` (only a user-confirmed region) ·
`meshtastic.set {device_id, key, value}` (allowlisted keys: `lora.region`, `lora.modem_preset`, `device.role` …) ·
`ble.pair {mac}` / `ble.forget {mac}` · `firmware.flash {device_id, image}`. Apply loop: snapshot → apply → probe
within 30 s → keep or revert, result written to the proposal. Unknown action = refused, never best-effort.

## 4. Options

### 4.1 Firmware default for user LoRa boards

_Outcome (D17 #1 + D19 c): option C with the 111 modem added — respect what is there; blank boards are offered the
111 modem (MeshCore KISS) first, RNode and Meshtastic as alternatives._

- **A. RNode everywhere** — board becomes an rnsd interface; one stack, host-side crypto/routing, `rnodeconf` already in
  the venv. Cost: reflash wipes the user's Meshtastic config; no interop with local Meshtastic users; deeper
  dependence on the Reticulum License (non-OSI field-of-use clauses incl. AI-training; we run it, do not train on it;
  https://github.com/markqvist/Reticulum/blob/master/LICENSE).
- **B. Meshtastic everywhere** — largest community; boards often arrive flashed. Cost: second stack and identity
  (`meshtastic-pki` history), flooding, our identity not end-to-end over the radio, bridge depends on Layer 5.
- **C. Respect what is there; RNode for blank boards (recommended).** Identify, never reflash silently: RNode → rnsd
  interface; Meshtastic/MeshCore → client child + read-only presence now, bridge later; blank → offer RNode (default)
  or Meshtastic with one sentence on each. Matches "existing networks are assets" and the rnsd we already run.
- **D. Reticulum over Meshtastic radios** — no reflash, ~500 B/s; experimental tier only.

### 4.2 Phone-as-relay transport

- **A. Wi-Fi AutoInterface** — zero-config; blocked on phones by MulticastLock until an app holds it; router client
  isolation breaks it.
- **B. TCP to hub (recommended first)** — hub runs a TCP server interface (LAN opt-in) + transport on; phone dials
  `111.local` or a QR. Works on Termux today; also how Columba connects.
- **C. BLE** — infrastructure-free, phone-to-phone via `ble-reticulum`; Linux side unconfirmed; experimental.
- **D. Phone as hotspot + B/A over it** — the literal "old phone's Wi-Fi connects others"; manual tethering first,
  LocalOnlyHotspot when an app exists.

### 4.3 Analog path

- **A. Detect and explain only (recommended for the opener)** — `analog` rows with honest copy and legal note; no driver.
- **B. Experimental tier later** — AIOC/Digirig + Dire Wolf + KISS, licensed operators, plaintext message class only
  (needs Layer 5 to define one), `id_callsign` enforced.
- **C. M17** — watch until M17-capable hardware is common; not a Baofeng-cable path.

### 4.4 Where device config lives

- **A. 111d renders from a declarative store (recommended).** `state/interfaces.toml` (bindings, params, posture) is
  the only thing actions touch; `111d` renders rnsd config `schema-version = 2` and client-child args. One writer,
  diffable, revertible, schema-checked.
- **B. Agent-written config proposals** — the proposal carries rnsd INI text; flexible, but free text from a 4B model
  into a daemon config is an injection and syntax hazard, and reverting means text diffs.
- **C. A + raw-stanza escape hatch** — advanced users paste a validated stanza; later, if asked.

### 4.5 Phone-side Android app (unlocks USB, BLE, multicast, hotspot)

- **A. Adopt Columba as the recommended phone app (recommended for roles b/c)** — MPL-2.0, BLE + TCP + RNode; it
  dials the hub over TCP. We write no APK. Enters KB as candidate; smoke test G5.
- **B. Build the D12 helper APK** — MulticastLock + USB-serial→localhost TCP (`usb-serial-for-android`, MIT) +
  LocalOnlyHotspot, serving the Termux node (role a). Custom code; keep scoped to what Columba cannot do.
- **C. Termux-only** — TCP to hub, nothing else; phones never drive radios. (Sequenced default: decision 3.)

## 5. Candidate KB entries (all `status: candidate`)

| Slug | Category | Why |
|---|---|---|
| `rnode-firmware` | project | LoRa board → Reticulum interface; default for blank boards (GPL-3.0) |
| `install-rnode-firmware-on-heltec-wifi-lora-32-v3` | recipe | `rnodeconf --autoinstall` path; sibling of the Meshtastic recipe |
| `meshtasticd` | project | Linux-native Meshtastic for SPI/CH341 radios only; scope note prevents misuse |
| `meshtastic-python` | project | client API/CLI over serial/BLE/TCP — the real driver of USB Meshtastic boards |
| `meshcore-cli` | project | MeshCore companion client (MIT); identify + read-only presence |
| `columba` | project | Android Reticulum/LXMF app, MPL-2.0; Sideband recorded under Alternatives (fails OSI gate) |
| `ble-reticulum` | project | BLE interface for Reticulum; license and Linux driver to verify |
| `rns-over-meshtastic` | project | Reticulum over unmodified Meshtastic radios; experimental; license to verify |
| `bluez` | project | Linux Bluetooth stack; pairing agent + D-Bus policy facts |
| `bluer` | project | Rust BlueZ binding for `111d` (license to verify) |
| `direwolf` | project | software TNC for the analog tier (GPL-2.0) |
| `usb-serial-for-android` | project | userspace USB-serial drivers if the helper APK is built (MIT) |
| `termux-api` | project | `termux-usb` fd semantics and limits (license to verify) |
| `lilygo-t-beam`, `lilygo-t-echo`, `rak4631`, `meshstick` | device | rows already in the LoRa table (MeshStick to disambiguate `1a86`) |
| `aioc` | device | audio/PTT cable for Baofeng-class radios |
| `baofeng-uv-5r` | device | the handheld; carries the certification and licensing notes |
| `kiss-tnc`, `ax-25`, `meshtastic-client-api` | protocol | framings the identify step and Dire Wolf speak |
| `m17` | protocol | watch list for open digital handhelds |
| `usb-serial-identification`, `amateur-radio-encryption-rule`, `lora-regional-parameters` | concept | the three facts the agent must never guess (findings 1, 6) |

New tags (`bluetooth`, `radio`, `ham`, `android`) go into `00-knowledge/_meta/vocabularies.md` first.

## 6. Registry contract check (additive only)

`DeviceRecord` lacks, in order of need:
- `match: {confidence: "exact"|"likely"|"unknown", rule: string}` — table vs probe vs nothing (finding 1).
- `firmware: {family: "rnode"|"meshtastic"|"meshcore"|"kiss"|"none"|"unknown", version: string|null,
  source: "probe"|"user"|"agent"} | null` — `capabilities` strings stay for compatibility.
- `driver: {stack: "reticulum"|"meshtastic"|"meshcore"|"dashboard"|null, interface: string|null, live: boolean}` —
  per-device binding; `service_id` keeps pointing at the registry service.
- `role: "controller"|"terminal"|"relay"|"node"|null` — companions (§2.2).
- `regulatory: {region: string|null, callsign: string|null, confirmed: boolean}`.
- `consent: {probe: boolean, at: date-time|null}` · `proposal_id: string|null` (open proposal targeting it) ·
  `busy_by: string|null` (port or BLE link held elsewhere).
- Enum growth: `DeviceTransport` + `spi`, `tcp` · `actions` + `identify`, `pair`, `authorize` · `reason_code` +
  `firmware_unknown`, `needs_region`, `needs_pairing`, `needs_permission`, `port_busy`, `needs_license`, `applying`.
- Rule for U1b/U6 now: unknown enum values render as `unknown`, never crash — makes all enum growth additive.
- Routes later: `POST /devices/{id}/identify`, `POST /devices/scan` (BLE, time-boxed), `POST /devices/{id}/pair`. *[Layer 5 (D20) builds `identify` now for the Meshtastic step — see the §3 amendment.]*

## 7. Owner decisions (decided 2026-10-14 — recorded as D17 in `DECISIONS.md`)

1. Firmware default — **offer flashing for blank boards only; a board that already runs something is left alone.** The
   dashboard must make it *super clear* what will be written and erased; the user decides (confirmation flow built in
   Layer 4: `flash_offer` + typed confirmation; the flashing core lands in Layer 6).
   **D19 (c) amendment (2026-10-14):** the blank-board offer lists the **111 modem (MeshCore KISS)** first, RNode
   (Reticulum) and Meshtastic as the alternatives, one sentence on each; "RNode makes the board an rnsd interface" now
   reads "a board is owned by 111d or by rnsd; rnsd attaches to a 111d-owned board only by loopback KISS on request."
2. Meshtastic coexistence — **ignore for now** (detect and name only); the bridge is parked in `cross-layer/parked.md` until the
   layers it needs exist. **Superseded in part by D18 #3 (2026-10-14):** identified + service row with its hosted panel +
   read-only presence into Nearby this cycle; the bridge stays parked (P1).
3. Phone app — **Termux** (it is the agentic path). Parked idea: once the agent reaches a phone it can install and try
   existing apps (Briar and the like), test their connectivity, make the phone a node there too, and bridge them with ours.
4. Phone-relay transport — **TCP to a hub first**, hotspot pattern second, AutoInterface when an app holds the lock,
   BLE experimental.
5. rnsd posture — **a user-choosable setting** (Settings → Layer 6 page) with the default: `share_instance = yes`
   everywhere; `enable_transport = yes` on Cat 3/4; TCP server only when LAN sharing is on.
6. Config home — **111d renders from `state/interfaces.toml`; the agent proposes typed actions.**
7. Applier timing — **yes to apply → verify → undo-on-failure, but not built now**; parked in `cross-layer/parked.md` for the
   layer that owns self-healing (Layer 9) or an earlier narrow cut if Layer 6 needs it.
8. Active identify — **one human consent per device, remembered; never automatic on plug-in.**
9. Firmware flashing — **human-confirmed only; images pinned in the signed manifest; no runtime downloads.**
10. Region and callsign — **asked once at first radio setup, stored node-level, per-device override; agent may suggest,
    never sets.**
11. Analog — **out of scope for now**; recorded in `cross-layer/parked.md` as the worked example for a later layer.
12. Reticulum dependence — **Reticulum is a connected service: used as-is and separately (own process, unmodified,
    always the latest upstream release), like Meshtastic. Our own light mesh core must not depend on it — if it ever
    does, plan a rewrite of our own in Rust. The boundary is stated in `os/LICENSING.md`.**

## 8. Sequencing

**Now — Layer 4 cycle (R4 + U6, passive only, no new risk):**
- R4: label table hits `match.confidence = likely`, `firmware.family = unknown`, reason `firmware_unknown`; define
  `needs_setup` as "present, no stack bound" (not "meshtasticd absent"); set `service_id: radio` only for SPI/CH341
  radios; detect USB audio + CDC pairs as `analog`.
- U6: tolerant enums; device detail shows "Identified as … (likely)" and "Driven by …"; Add page splits phones into
  "control this node" (pair, exists), "make it a node" (install command), "message through this node" (coming with
  Layer 6); handheld card states licensed + plaintext-only.
- Flag to Layer 4 owner: the `radio` service row (§7) is meshtasticd-only; USB Meshtastic boards need it reworded. *[Resolved by Layer 5 (D20): the radio-row rule counts a `meshmud`-bound board as bound (messaging §14).]*

**Layer 6 opener — device-connection core (in order):**
1. `111d` `devices/` module: uevent hotplug, port ownership lock, ModemManager/brltty guards, consented `identify`
   via adopted CLIs in the rns venv. *[The Meshtastic step is built by Layer 5 (D20): see the §3 amendment.]*
2. Declarative `state/interfaces.toml` + rnsd renderer `schema-version = 2` (RNodeInterface stanzas, posture, LAN TCP
   server); region table from the KB concept entry.
3. Agent tools (`devices.read`, `transport.read`, `device.identify`, `device.propose`, `service.restart`) + the narrow
   applier with verify/auto-revert; proposals render as device cards in the UI.
4. Phone relay v1: TCP to hub, QR join, `role` on companions; Termux node gains a hub uplink.
5. Meshtastic/MeshCore client children, read-only presence into Nearby. *[The Meshtastic child (`meshmud`) and its presence reader are built by Layer 5 (D18 #3, D19 g); MeshCore stays here.]*

**Later:** Columba adoption or helper APK (MulticastLock, USB→TCP, LocalOnlyHotspot) · BLE pairing UI + RNode/Meshtastic
over BLE · firmware flashing from manifest-pinned images · message bridges (after Layer 5) · analog experimental tier ·
owned-node linking (Layer 2 attestation + Layer 7) · SPI HAT meshtasticd install path.

**Verification gates before the opener locks:** G1 `meshtastic --info` and `rnodeconf --info` over
`/dev/serial/by-id` on Heltec V3 + T-Beam without unwanted resets · G2 `rnodeconf --autoinstall` offline from local
images · G3 Termux rnsd `TCPClientInterface` to a hub reaches `joined` without multicast · G4 Android hotspot
client-to-client traffic + AutoInterface on two OEMs · G5 Columba ↔ hub rnsd over TCP · G6 BlueZ pairing of a BLE
RNode as `node111` under the systemd sandbox.

## 9. Layer 5 hand-offs (D20, 2026-10-14)

Pinned in `os/50-messaging/README.md` and owned by Layer 6 at kickoff: the **`Link` trait** (§8: `PathProfile` with
`mtu_bytes`, framing, `reliable`, `broadcast`, `cost_class`, advertised airtime; `PeerAddr`; boxed-future `send`/`recv`)
is the contract every driver here implements · the **MeshCore data-type range** for 111 `GRP_DATA` frames is registered
with upstream (`number_allocations.md`) before this layer locks (D19 c) · **INTRO over LoRa** (card in fragment 0, a
capacity-bounded pending-INTRO buffer) is deferred to this layer (messaging §21) · `interfaces.toml` is born here and
inherits the Meshtastic binding from `devices.json`; the full identify sequence (RNode, Meshtastic, MeshCore) reuses the
daemon port-owner table.
