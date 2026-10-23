# Layer 6, spike S5: Delta Chat (`deltachat-rpc-server` + Madmail) actually run (2026-10-19)

Evidence tags: **run** = executed on the build box (x86_64, Ubuntu glibc 2.39, 4 CPUs, no hardware); **src** = read from the pinned source tree
(Madmail at the tag below, chatmail core at the tag below); **doc** = upstream docs only. Nothing ships from this spike. Scratch lived under the
session scratchpad and was deleted; nothing is left running (see section 9).

**Spike shape (deviation from §7's wording, on purpose):** the box has no `ip` binary, so no veth. Madmail and both `deltachat-rpc-server` processes
ran on loopback inside **one** unprivileged network namespace (`unshare -Urnm`; `lo` raised with a 10-line `SIOCSIFFLAGS` ioctl; no other
interface, empty routing table). That proves "no internet" (negative control: `connect(1.1.1.1:443)` from inside → `ENETUNREACH`, errno 101). The
two-netns veth form, with the hub's LAN address in the account addresses, stays the runbook's (§16).

## 0. Pins

| Program | Pin | sha256 | Size | Notes |
|---|---|---|---|---|
| `deltachat-rpc-server` linux x86_64 | https://github.com/chatmail/core/releases/download/v2.63.0/deltachat-rpc-server-x86_64-linux · tag `v2.63.0` → commit `de6f46971002c64b293d6095e66d8a1b7a11fd43` | `57cab3291ecc55736f97964b3a2979e5e23a70abcc270f1ed515e995f540d517` | 26,811,824 B | static-pie, stripped; `--version` → `2.63.0` |
| `deltachat-rpc-server` linux aarch64 | …/v2.63.0/deltachat-rpc-server-aarch64-linux | `040b0c3699361c9aa9776caf259d73b70c5892937f6f85836e26143ba1cdbd13` | 21,009,952 B | static, stripped (not run) |
| `deltachat-rpc-server` Android arm64 | …/v2.63.0/deltachat-rpc-server-arm64-v8a-android | `3e2d0c9306fccee78ccbd5b74c73430ed35dd1b31b31931abba97e8bbf9d658c` | 21,897,944 B | bionic linker64 (not run) |
| Madmail linux amd64 | https://github.com/themadorg/madmail/releases/download/v2.30.5/madmail-linux-amd64 · tag `v2.30.5` → commit `22db4602768364955670b0f08dd97f03db6e22cc` | `ad89fb23de9580cd736c7caa8136422415cb8779954c16e366d94f9325b989d1` | 77,331,312 B (61,282,520 stripped) | **release binary used, no cargo build**; dynamic glibc, not stripped; `version` → `madmail-v2 2.30.5` |
| Madmail linux arm64 | …/v2.30.5/madmail-linux-arm64 | `27e6e74d9a268f33bb23a12aa57e5bcb1e164f3d670e6fe3d2b2bb7e541f20b5` | 73,479,752 B | dynamic glibc (not run) |

- Hashes are of the bytes downloaded over the proxy (`curl -L -o`, `sha256sum`); neither project publishes a checksum file for these assets (Madmail:
  `SHA256SUMS`, `sha256sums.txt`, `checksums.txt`, `*.sha256` all 404; the GitHub API digest was not reachable from this session). The signed
  111 manifest becomes the first independent record (§2.1).
- **Madmail tags ≠ releases.** Tags run to `v2.35.2` / `v2.37.0-unstable` (semantic-release), but assets exist only on published releases;
  `releases/latest/download/madmail-linux-amd64` resolves to **v2.30.5** and `…/v2.35.2/madmail-linux-amd64` is 404. Pin by release, not tag.
- No armv7 asset from either project (both 404). 32-bit Pi OS is out for this component.
- Licences read before pinning: Madmail `LICENCE` = GNU AGPL v3 (README: AGPL-3.0-or-later); chatmail core MPL-2.0 (R03 [S8]).

## 1. Summary

- **PASS.** In a namespace with no route off the box, Madmail v2.30.5 started in IP-only, self-signed mode; two headless `deltachat-rpc-server`
  v2.63.0 accounts (a bot-flagged "hub" and a "phone") configured against it, completed Secure-Join (QR text from one consumed by the other, both
  sides reached progress 1000), exchanged a message each way end-to-end encrypted, and the core reported pending → delivered → read. Three full runs,
  the third with Madmail on **unprivileged ports, loopback-bound, with `CAP_NET_BIND_SERVICE` removed**.
- **Egress: zero.** Madmail made **0** `connect()` calls in ~10 min across both configs; each rpc-server made exactly 4, all to `127.0.0.1` on the
  relay's IMAP/SMTP ports. The only interface is `lo`.
- **Ports:** the IP-only install serves SMTP 25, submission 465 (TLS) + 587, IMAP 993 (TLS) + 143, HTTP 80, HTTPS 443, TURN 3478/udp, Shadowsocks
  8388, OpenMetrics 127.0.0.1:9749. **Every listen address is configurable**; on ports ≥ 1024 no capability is needed (run). With the standard
  ports the unit needs `CAP_NET_BIND_SERVICE` (run: `cannot listen on 0.0.0.0:25: Permission denied`).
- **Traps found:** (1) the bot flag turns **read receipts off** unless `mdns_enabled=1` is set explicitly; (2) `install` writes a man page and shell
  completions into `/usr/share` even with scratch paths, so the component must not run `install`; (3) the simple install enables Shadowsocks, TURN,
  plain HTTP and binds `0.0.0.0`, and `push auto` talks to `notifications.delta.chat` when a phone registers a token; (4) the release binary needs
  **glibc ≥ 2.38**, so it does not run on Debian 12 / Pi OS bookworm (2.36) or Ubuntu 22.04; (5) Madmail's IMAP decreases `UIDNEXT`, and the core
  warns and resyncs on every fetch (works, noisy).

## 2. What ran (commands)

```sh
# netns holder (lives until killed; the netns dies with it)
unshare -Urnm --fork sleep 7200 &   P=$!
nsenter -t $P -U -n -m --preserve-credentials python3 -I loup.py      # SIOCSIFFLAGS lo up
# Madmail IP-only, self-signed, into scratch paths (no systemd, no user)
madmail install --simple --ip 127.0.0.1 -n --tls-mode self_signed --no-obtain-certificate \
  --config-dir $S/mm/etc --state-dir $S/mm/state --skip-systemd --skip-user --binary-path $S/mm/bin/madmail
#   → DKIM skipped ("d= cannot be an IP literal"), self-signed ECDSA P-256 cert, domain "[127.0.0.1]"
#   runtime_dir edited from /run/madmail to scratch
nsenter … strace -f -tt -e trace=connect,bind,sendto,sendmsg madmail --config $S/mm/etc/madmail.conf --state-dir $S/mm/state run
nsenter … madmail --config … --state-dir … --json create-user     # one per account → dclogin: URI
# driver: stdlib Python, spawns two `strace -f -e trace=connect,sendto,sendmsg,bind deltachat-rpc-server`
#   (DC_ACCOUNTS_PATH per peer), JSON-RPC 2.0 over their stdin/stdout, logs every line both ways and every event
nsenter … python3 -I drive.py $S "$DCLOGIN_HUB" "$DCLOGIN_PHONE"
# capability check (run 3): drop the capability from the bounding set, so the exec'd root has CapEff 000001fffffffbff
nsenter … capsh --drop=cap_net_bind_service -- -c "madmail --config madmail.conf … run"      # → EACCES on :25
nsenter … capsh --drop=cap_net_bind_service -- -c "exec strace … madmail --config madmail-hi.conf … run"   # high ports → starts
```

Driver call sequence (per peer): `get_next_event_batch` (long-poll, kept outstanding) · `get_system_info` · `add_account` · `set_config bot 1`
(hub) · `set_config displayname …` · [`set_config mdns_enabled 1` (hub, runs 2–3)] · `add_transport_from_qr <dclogin:…>` · `is_configured` ·
`list_transports` · `start_io` · `wait_for_all_work_done` · hub `get_chat_securejoin_qr_code(acct, null)` · phone `check_qr` · phone
`secure_join` · wait `SecurejoinInviterProgress`/`SecurejoinJoinerProgress` 1000 · `misc_send_text_message` · poll `get_message().state` ·
wait `IncomingMsg` · `get_message` / `get_contact` · `markseen_msgs` · `get_message_read_receipts` · `get_message_info` · stdin close (the server
exits on EOF).

## 3. Results

| # | Step | Result | Evidence (run 3 unless noted) |
|---|---|---|---|
| 1 | Obtain binaries | **PASS** (run) | Release assets downloaded and hashed (section 0); no cargo build needed |
| 2 | Madmail IP-only self-signed install | **PASS, with trap** (run) | Config `$(primary_domain) = [127.0.0.1]`, `tls_mode self_signed`, DKIM skipped. strace of `install` shows writes to `/usr/share/man/man1/madmail.1`, `/usr/share/bash-completion/completions/madmail`, `/usr/share/zsh/site-functions/_madmail` (dir created), `/usr/share/fish/vendor_completions.d/madmail.fish` despite non-system paths ("Skipping binary install (non-system paths)"). Removed by hand after the run |
| 3 | Madmail starts offline | **PASS** (run) | All listeners up < 1 s; 0 `connect()`; only `AF_NETLINK` `RTM_GETLINK`/`RTM_GETADDR` at boot. RSS 34.6–35.6 MB |
| 4 | Account creation | **PASS** (run) | `madmail --json create-user` → `{"dclogin":"dclogin:2f6zraev6xd4@[127.0.0.1]/?p=…&v=1&ih=127.0.0.1&ip=993&is=ssl&sh=127.0.0.1&sp=465&ss=ssl&ic=3", "email", "password", "username"}`; the URI follows the configured ports (`ip=9993&sp=4465` in run 3). Open registration also answers `POST https://<ip>/new` with the same JSON (`dclogin_url`) |
| 5 | Core configures against a self-signed IP relay | **PASS** (run) | `add_transport_from_qr(dclogin)` → `null`; `list_transports` → `certificateChecks: "acceptInvalidCertificates"` (from `ic=3`), IMAP 993 ssl, SMTP 465 ssl, server `127.0.0.1`. `ConfigureProgress` events 0…1000 |
| 6 | Bot flag | **PASS** (run) | `get_config bot` → `"1"`; the phone sees the hub's messages with `isBot: true`; the core adds `Auto-Submitted: auto-generated` (src `mimefactory.rs:1141`) |
| 7 | Secure-Join | **PASS** (run) | Hub QR text: `https://i.delta.chat/#EDDC26488B55F1335DE32B36854DEAA61FE882F8&v=3&i=…&s=…&a=2f6zraev6xd4@%5B127.0.0.1%5D&n=111+bridge` (parsed locally, never fetched). Phone `check_qr` → `kind: "askVerifyContact"`, `is_v3: true`. Joiner progress 400 → 1000, inviter 1000 at +0.3 s; chat auto-accepted on both sides (no `accept_chat` needed) |
| 8 | Phone → hub | **PASS** (run) | Phone state 20 → 26 in 56 ms; hub `IncomingMsg` → `text: "hello hub from phone", state 10, showPadlock: true`; sender contact `isKeyContact: true, e2eeAvail: true, isBot: false` |
| 9 | Hub → phone | **PASS** (run) | Hub state 20 → 26 in 56 ms; phone `IncomingMsg`, `isBot: true, showPadlock: true`; `get_message_info` → "State: Delivered, Encrypted … Message-ID: …@localhost" |
| 10 | Read receipt to a normal account | **PASS** (run) | After hub `markseen_msgs`, phone msg 26 → 28 in 112 ms; `MsgRead` event; `get_message_read_receipts` → `[{contactId:10, timestamp}]` |
| 11 | Read receipt to the bot | **FAIL by default, PASS with `mdns_enabled=1`** (run 1 vs runs 2–3) | Run 1: the hub's message stayed 26 for the full 30 s after the phone marked it seen. Cause (src `config.rs:614-618`): `should_request_mdns` defaults to `!Bot`. With `mdns_enabled=1` on the hub: 26 → 28 in 112 ms, `MsgRead` on the hub |
| 12 | Egress | **PASS** (run) | Section 5 |
| 13 | Unprivileged ports, no capability | **PASS** (run) | Run 3 config: SMTP 127.0.0.1:2525, submission 4465/5587, IMAP 9993/1143, HTTP 8080, HTTPS 8443, Shadowsocks lines removed. Under `capsh --drop=cap_net_bind_service` it starts and the whole flow (rows 4–11) passes |
| 14 | Same, standard ports | **FAIL as expected** (run) | `chatmail: error: cannot listen on 0.0.0.0:25: Permission denied (os error 13)`, exit 1 |
| 15 | Weight | measured (run) | rpc-server RSS 28.5–30.9 MB per process with one account; account dir 544–668 KiB after the runs; Madmail RSS ~35 MB, state 1.2 MiB |

## 4. Ports and the capability verdict

| Service | `install --simple --ip` default | Config key (maddy-style `.conf`) | Spike run 3 |
|---|---|---|---|
| SMTP (MX, inbound federation) | `tcp://0.0.0.0:25` | `smtp tcp://…` | 127.0.0.1:2525 |
| Submission | `tls://0.0.0.0:465`, `tcp://0.0.0.0:587` | `submission …` | 4465 / 5587 |
| IMAP | `tls://0.0.0.0:993`, `tcp://0.0.0.0:143` | `imap …` | 9993 / 1143 |
| HTTP (`/new`, web) | `tcp://0.0.0.0:80` | `chatmail tcp://…` | 8080 |
| HTTPS (`/new`, web; ALPN smtp/imap on the same port) | `tls://0.0.0.0:443` | `chatmail tls://…` | 8443 |
| TURN | `udp+tcp 0.0.0.0:3478` (tcp did not listen) | `turn …` | 3478/udp (still bound `0.0.0.0` although configured `127.0.0.1`) |
| Shadowsocks | `0.0.0.0:8388`, **on by default** | `ss_addr` in each `chatmail` block | removed |
| OpenMetrics | `127.0.0.1:9749`, no auth | `openmetrics …` | same |

**Verdict.** Madmail does **not** require `CAP_NET_BIND_SERVICE`: every listener moves, and a client learns the ports from the `dclogin:` URI that
Madmail itself emits. On the **standard** ports (25/143/465/587/993/80/443) it does need it, and Termux cannot have them. Recommendation for C5:
- **Loopback-only mode (hub bridge account only):** high loopback ports, no capability. Proposed (draft) block: SMTP 11240, submission 11241,
  IMAP 11242, HTTPS 11243, in the 111 range next to meshmud 11230 and SimpleX 11232; TURN, Shadowsocks, plain HTTP and port 9749 off. Claim and
  pre-bind each in the `PortTable` (§2).
- **LAN sharing on (phones join):** a phone's Delta Chat reads non-standard ports from a `dclogin:`/`/new` answer, so high LAN ports are plausible
  without the capability (*unverified on a phone*: part of the attended gate). Standard ports only if the owner wants federation with outside relays
  (port 25), then `AmbientCapabilities=CAP_NET_BIND_SERVICE` on the unit (§18). The §2 line "Madmail serves the standard chatmail ports … which need
  `CAP_NET_BIND_SERVICE`" should be corrected to "configurable; the standard ports need the capability".

## 5. Egress log

Single namespace, interfaces: `lo` only, routing table empty. `strace -f -tt -e trace=connect,bind,sendto,sendmsg` on Madmail (two configs) and on
both rpc-servers in all three runs; `/proc/net/dev` before and after.

| Program | Window | `connect()` total | Targets |
|---|---|---|---|
| Madmail (standard ports) | ~5 min, runs 1–2 plus idle 60 s | **0** | none; `sendto` only to `AF_NETLINK` (4, interface/address dump at boot) |
| Madmail (high ports, no capability) | ~4 min, run 3 plus idle 30 s | **0** | none |
| rpc-server hub / phone (each run) | each run incl. idle | **4** each | `127.0.0.1:993` ×2, `127.0.0.1:465` ×2 (run 3: `:9993`, `:4465`) |
| Interfaces | whole session | n/a | `lo` only (run 1: 34 → 366 packets; run 2: 376 → 865); there is no other interface to leak on |

Static scan (src, Madmail v2.30.5 `crates/**/*.rs` string literals) of what could reach outside when triggered: `https://notifications.delta.chat/notify`
(push, when a client registers a device token over IMAP `SETMETADATA /private/devicetoken`; `madmail push off` disables), `api.github.com/repos/themadorg/madmail/releases`
and `github.com/…/releases/latest/download/` (the `upgrade` command only), `admin.madmail.chat` (admin-web link), ACME (only with `--auto-ip-cert`
or `certificate acme`). The headless core registered no push token, so none fired. The C5 component must set `push off` and never run `upgrade`.

## 6. Message-state mapping observed

Core `MessageState` ints (src `deltachat-rpc-client/const.py`): 20 `OUT_PENDING`, 24 `OUT_FAILED`, 26 `OUT_DELIVERED`, 28 `OUT_MDN_RCVD`; inbound
10 `IN_FRESH`, 13 `IN_NOTICED`, 16 `IN_SEEN`.

| Observed on the sender | Event | Meaning (run) | Proposed `ForeignMessageState` (§4) |
|---|---|---|---|
| 20 right after `misc_send_text_message` | none | queued in the core's SMTP queue | `queued` ("pending") |
| 26 after ~56 ms | `SmtpMessageSent` ("…SMTP-sent to 1 recipients") then `MsgDelivered {chatId,msgId}` | **the relay accepted it over SMTP**; nothing about the recipient | `sent`, evidence "the relay accepted it". The core has no separate "handed to the relay" step, so §4's draft Delta Chat row collapses `sent` and `relayed` into one; use `sent` |
| 28 after the peer's `markseen_msgs` | `MsgRead {chatId,msgId}` + `MsgReadCountChanged` | the recipient's read receipt (MDN) | `delivered`, evidence "the recipient's app sent a read receipt" (may never come: the peer can disable receipts) |
| 24 | `MsgFailed` | not observed | `failed` |

- `MsgDelivered` also fires for the hidden Secure-Join handshake messages (msg ids 13, 15 before the first text), so the adapter keys by the msg id
  that `misc_send_text_message` returned and ignores others.
- 26 and 28 are monotone in every run; nothing regressed.
- For a bridge account, `delivered` exists only if 111 sets `mdns_enabled=1` (row 11). That also makes the hub **send** read receipts when it marks
  a message seen; marking is 111's choice (mark on ingest, or only when shown in Messages).

## 7. What the C5 adapter needs from the stdio protocol

- **Framing:** one JSON object per line, both directions; requests `{"jsonrpc":"2.0","method":…,"params":[positional…],"id":n}`. Responses come
  **out of order** (a pending `get_next_event_batch` with id 1 is answered after id 2, 3 …), so the reader matches by id. stdout carries only
  JSON-RPC; logs go to stderr. Closing stdin makes the server exit cleanly (used for shutdown in every run). `DC_ACCOUNTS_PATH` picks the accounts
  dir (else the cwd); 111 sets it to `bridge/deltachat/accounts` (0700).
- **Events:** keep one `get_next_event_batch` outstanding at all times (it long-polls); the result is
  `[{"contextId": <account id>, "event": {"kind": "<Kind>", …}}]`. Kinds the adapter uses: `IncomingMsg {chatId,msgId}`, `MsgDelivered`,
  `MsgRead`, `MsgFailed`, `SecurejoinInviterProgress {contactId,chatId,chatType,progress}` (1000 = done: the contact's opt-in act), `ConfigureProgress`,
  `ConnectivityChanged`, `Warning`/`Error` (log). `Info` is ~200 events per session and should be dropped at the reader.
- **Create and configure the bridge account:** `add_account()` → id; `set_config(id,"bot","1")`; `set_config(id,"mdns_enabled","1")`;
  `set_config(id,"displayname",…)`; `add_transport_from_qr(id,"dclogin:…")` (from `madmail --json create-user`, or `POST /new`); `start_io(id)`;
  `wait_for_all_work_done(id)` before first use. `get_system_info()` is the health call (`deltachat_core_version`).
- **Secure-Join QR for the card:** `get_chat_securejoin_qr_code(id, null)` → text (above); `get_chat_securejoin_qr_code_svg(id, null)` gives
  `[text, svg]` for the UI (src, not run). It is an `https://i.delta.chat/#…` URL that the app parses offline.
- **Send / receive / read:** `misc_send_text_message(id, chatId, text)` → msgId; `get_message(id, msgId)` → `{text, state, isBot, showPadlock,
  fromId, chatId, viewType, …}`; `get_contact(id, contactId)` → `{address, displayName, authName, isBot, isKeyContact, e2eeAvail, …}` (there is
  no `isVerified` key in v2.63.0); `markseen_msgs(id,[msgId])`; `get_message_read_receipts(id,msgId)`.
- **Retention:** `delete_device_after` (seconds, default `"0"`) is **account-wide**, not per chat (src `config.rs:192-198`). The §7 shape "for
  non-bridged chats" is not available as such; per-chat deletion exists only as the chat's ephemeral timer, which is visible to and shared with the
  peer. C5 sets `delete_device_after` account-wide (bridged texts are copied into Messages first) or documents the residual.

## 8. Residuals (what a phone and a real hub still need)

1. **Attended gate (unchanged, §16):** whether the Android/iOS Delta Chat apps accept an IP-only, self-signed relay on a LAN address
   (`ic=3` in the `dclogin:` should make them accept the certificate), and whether they accept **non-standard ports** from a `dclogin:`/`/new`
   answer. Not provable without a phone.
2. **Two-netns veth form** with a real LAN address (`user@[192.168.x.y]`) and LAN-bound listeners: the runbook form; the spike's loopback form shows
   the protocol path, not the LAN binding.
3. **glibc floor:** the Madmail release binary needs glibc ≥ 2.38 (strong symbols `__isoc23_strtol`/`__isoc23_strtoul@GLIBC_2.38`; `pidfd_*@2.39`
   are weak). Runs on Debian 13 / Pi OS trixie (2.41) and Ubuntu 24.04 (2.39); **not** on Debian 12, Pi OS bookworm (2.36) or Ubuntu 22.04. No armv7
   asset. A Pi on bookworm needs a CI build (older-glibc or musl target) or the trixie floor: owner's call. The rpc-server is static and has no floor.
4. **Weight:** Madmail 77 MB unstripped (61 MB stripped), ~35 MB RSS; rpc-server 27 MB, ~29 MB RSS. Zero when disabled (§2.1), but ~64 MB RSS while
   on, which matters on a Pi-class box.
5. **Run Madmail without `install`:** the component unpacks the binary and writes its own config (no `/usr/share` writes, no systemd), with
   `runtime_dir` and `state_dir` under `components/madmail/` and `bridge/deltachat/`; `push off`; Shadowsocks, TURN, plain HTTP and OpenMetrics off;
   listeners bound to loopback, or to the LAN address only while LAN sharing is on.
6. **IMAP `UIDNEXT` warning:** "The server illegally decreased the uid_next of folder INBOX from N to 1 without changing validity, resyncing UIDs" on
   every fetch, both peers, all runs. Delivery still works; worth an upstream issue before C5 relies on it.
7. **Upstream maturity:** Madmail v2 is a young Rust rewrite (README: "Most of the Rust … was written with AI assistance"; "Use at your own risk").
   The pin moves only by a reviewed release, never by its `upgrade` command.

## 9. Hygiene

`kill` of Madmail and of the namespace holder; afterwards `ps -eo pid,comm,args | grep -E "madmail|deltachat|strace|unshare|nsenter|sleep 7200"`
shows nothing and `ss -ltnup` shows only the session's own listeners (127.0.0.1:33163, 127.0.0.1:38419, :2024, none ours). The stray files from
`madmail install` in `/usr/share` were removed (four files and the created `/usr/share/zsh/site-functions`). The scratch directory, including
the Madmail and core source clones, the binaries and all logs, was deleted.

## VERDICT

**PASS: Delta Chat stays in R-B. The C5 row builds against these ports: Madmail on configurable loopback ports ≥ 1024 with no capability (the
standard ports only with `CAP_NET_BIND_SERVICE`), `deltachat-rpc-server` v2.63.0 over piped stdio, with `bot=1` plus `mdns_enabled=1`.**

## Sources

https://github.com/chatmail/core (tag v2.63.0: `deltachat-rpc-server/README.md`, `deltachat-jsonrpc/src/api.rs`, `api/types/login_param.rs`,
`deltachat-rpc-client/src/deltachat_rpc_client/{rpc,account,chat,const}.py`, `src/config.rs`, `src/mimefactory.rs`) ;
https://github.com/themadorg/madmail (release and tag v2.30.5: `README.md`, `LICENCE`, `chatmail.toml.example`, `docs/local-dev.md`,
`docs/project/user-guide/11-deployment-ip-domain-certs.md`, `crates/chatmail-config/src/{lib,client_mail}.rs`, `.github/workflows/ci.yml`,
`.github/releaserc.ci.json`). Repo: `6-transport/README.md` §2, §4, §7, §15.10; `plan/layers/6-transport/research/03-phone-apps.md` §3.9.

## Addendum 2026-10-23 — review citations

The C5 review (fable, 2026-10-23) verified these upstream facts by reading source at the pinned tags; they are recorded here from the
review's text (nothing was fetched or run for this addendum). They cover what C5 builds on beyond what this spike ran.

**Core v2.63.0 (`deltachat-rpc-server`, `deltachat-jsonrpc`).**

- The 18 JSON-RPC methods C5 calls exist: `get_system_info`, `get_all_account_ids`,
  `add_account`, `set_config`, `get_config`, `is_configured`, `add_transport_from_qr`, `start_io`, `wait_for_all_work_done`,
  `get_chat_securejoin_qr_code`, `get_next_event_batch`, `get_message`, `get_contact`, `get_basic_chat_info`, `get_chat_id_by_contact_id`,
  `create_chat_by_contact_id`, `misc_send_text_message`, `markseen_msgs`.
- All 53 `KNOWN_KINDS` exist in `api/types/events.rs` (the review's count; the list in `N/bridge/deltachat.rs` as of this
  addendum has 52 entries, every one of them covered), with camelCase fields (the ones C5 reads: `msgId`,
  `chatId`, `contactId`, `progress`, `n`).
- `chatType` serialises as `"Single"` / `"Group"`.
- `MessageObject` carries `fromId`, `chatId`, `state` (u32), `isInfo`, `fileBytes`, `timestamp` and `sender`; `ContactObject` carries
  `address`, `displayName` and `authName`.
- (C5-bounce, not part of the review's verified list) the receive gate also refuses `showPadlock == false` when the message object carries
  it (`MessageObject.show_padlock`); without the field the contact-id gate alone decides.

**Madmail v2.30.5.**

- The TOML keys C5 renders (`hostname`, `primary_domain`, `state_dir`, `runtime_dir`, `tls_mode`, `debug`, `auth_auto_create`,
  `smtp_listen`, `submission_tls_listen`, `imap_tls_listen`, `http_tls_listen`, `turn_enable`) exist in `AppConfig`.
- `--config *.toml` is parsed as TOML.
- `effective_primary_domain` wraps an IP as `[x.x.x.x]`, so account addresses read `user@[192.168.1.5]`.
- `tls_boot.rs` self-generates the certificate for `tls_mode = "self_signed"`.
- `--json` is a global flag, and `create-user`, `push off`, `federation policy reject` and `registration open|close` are real subcommands.

**What this spike did not run.** The spike ran the maddy-style `.conf` that `madmail install` writes (§2), not the TOML form C5 renders,
and it never ran `federation` or `registration`. C5's integration test drives a shell fake of the relay that accepts any verb, so those
two subcommands and the TOML form are verified by source reading only; G7 runs the real binaries once.
