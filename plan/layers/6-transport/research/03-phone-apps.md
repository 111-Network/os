# Lens 3 — Phone apps via Termux (parked P2): what is honestly feasible

Scout report, 2026-10-19. Repo facts cite `file:line`; web facts cite the URL in section 6 (`[Sn]`). "(unverified)" = not confirmed from an upstream file. Hardware: none on the lead box, so nothing here was run on a phone.

## 1. Summary

- **A Termux agent can do less to a phone than the P2 wording hopes.** Termux:API gives sensors, notifications (including *reading* other apps' notifications), SMS, clipboard, share, SAF storage and `termux-open`. It has no UI driving, no silent APK install and no Bluetooth. Driving another app's UI needs ADB wireless debugging to the same phone, a human-paired, unreliable channel that breaks on screen lock, reboot, Wi-Fi change and (Android 17) untrusted networks.
- **Briar cannot be an offline bridge target from a hub.** `briar-headless` registers only the Tor plugin on Linux [S2]. The offline transports (Bluetooth, Wi-Fi LAN) exist only in the Android app (no API) and in Briar Desktop (GUI, no API) [S5]. A hub can be the phone user's Briar contact only over Tor, so with internet on both sides. That is a gateway-class bridge.
- **SimpleX and Delta Chat are the two apps with a first-class headless peer plus a LAN-capable self-hosted server.** SimpleX has a documented bot WebSocket API and `smp-server init --ip` for LAN [S10]. Delta Chat has `deltachat-rpc-server` (JSON-RPC over stdio) and an experimental IP-only chatmail relay [S7][S9]. Delta Chat is the **only** candidate with an Android arm64 build, so a Termux phone can itself be a node there.
- **The agent cannot be the actor.** `agent111` is "no shell, no plugins, ever" (`3-intelligence/crates/agent111/src/tools.rs:1`) and `messages.send` only writes a proposal (`tools.rs:13-15`). Any ADB or Termux action must be a typed, human-approved action owned by `111d` (parked P4), not an agent tool.
- **Recommend:** v1 "phone apps" promises a small app-peer adapter shape with two reference adapters (SimpleX, Delta Chat). Briar is documented as online-only. ADB or UI driving is a user-attended, KB-guided experiment, off by default, outside the v1 promise.

## 2. Candidates

| Name | Upstream | License (verified?) | Runtime | Maturity (dates) | Fit | Why |
|---|---|---|---|---|---|---|
| SimpleX Chat CLI + `smp-server` | github.com/simplex-chat/simplex-chat, simplexmq | AGPL-3.0 (yes, LICENSE file [S10]) | Haskell, glibc Ubuntu 22.04/24.04 x86_64 + aarch64 binaries; no Android CLI asset | CLI v7.0.3 2026-09-26; simplexmq v7.1.0-beta.4 2026-10-17; 19.6k stars | **bridge** (tier i) | Bot API is documented and first-class; LAN server via `--ip`; separate process only (AGPL) |
| Delta Chat: `deltachat-rpc-server` + chatmail relay | github.com/chatmail/core, chatmail/relay | core MPL-2.0 (yes, LICENSE + Cargo.toml); relay MIT (yes) | Rust; prebuilt linux x86_64/aarch64/armv7, **android arm64-v8a**; relay = Debian 12 + Postfix/Dovecot/Nginx | core v2.63.0 2026-10-09; relay 1.13.0 2026-09-22; Android app v2.62.0 2026-09-26 | **bridge** (tier i) | Lightest integration (stdio JSON-RPC, `Bot` config flag); runs in Termux; LAN needs the heavy relay |
| briar-headless | github.com/briar/briar (mirror of code.briarproject.org) | GPL-3.0-or-later per LICENSE.txt notice (yes); KB says `GPL-3.0-only` (discrepancy) | Kotlin/JVM jar (Java 8+), Linux x86/aarch64/armhf | Briar 1.5.22 2026-10-09; headless commits are Tor bumps only (2026-10-09) | **bridge**, online-only | REST + WS API works, but Tor-only on Linux; honest tier (i) only with internet |
| Briar Desktop | github.com/briar/briar-desktop | AGPL-3.0 (GitHub metadata only; unverified from file) | Kotlin Compose GUI | 0.6.5-beta, mirror pushed 2026-08-16 | **reference** | Has LAN plugin but no API; GUI only |
| Jami (`jami-daemon`) + OpenDHT | github.com/savoirfairelinux/jami-daemon, opendht | daemon GPL-3.0-or-later (yes, file header); OpenDHT MIT (API metadata; unverified from file) | C++ daemon, D-Bus API (`sendMessage`, `getConversations`) | daemon pushed 2026-10-17; OpenDHT v4.4.0 2026-09-04 | **reference** | Works on a LAN with a bootstrap node, but offline reports of failures and a heavy C++ build |
| Berty / Wesh | github.com/berty/berty, weshnet | Apache-2.0 OR MIT (yes, README + LICENSE-APACHE) | Go; `berty daemon` gRPC | pushed 2026-10-16; last release v2.471.14 2026-07-03 | **reference** | README: protocol "partially implemented", API unstable; BLE only in mobile app |
| Bitchat | github.com/permissionlesstech/bitchat, bitchat-android | iOS: Unlicense (yes); Android: README says public domain but `LICENSE.md` is GPL-3.0 (yes, discrepancy) | Swift / Kotlin, BLE mesh (+Wi-Fi Aware, Nostr) | v1.7.4 2026-06-17; android v2.0.1 2026-08-11 | **reject** for v1 | No API or headless peer; BLE needs a custom Rust implementation |
| Matrix: Tuwunel + matrix-rust-sdk | matrix-construct/tuwunel, matrix-org/matrix-rust-sdk | Apache-2.0 both (API metadata; unverified from file) | Rust | Tuwunel v1.9.4 2026-10-10; SDK 0.19.1 2026-09-18; matterbridge stale (pushed 2024-12-12) | **reference** (note only) | Best in-binary Rust option after D22, but out of lens; phones need Element pointed at a LAN homeserver |
| Termux + Termux:API | github.com/termux/termux-app, termux-api, termux-api-package | app GPLv3-only (yes); Termux:API GPLv3 (README); api-package MIT (yes) | Android app + CLI scripts | app v0.118.3 2025-05-22 (beta .119 same day); Termux:API v0.53.0 2025-09-01; package v0.60.0 2026-09-16 | **adopt** (already bundled, `1-bootstrap/README.md:40`) | The only no-root phone-side runtime |
| `android-tools` (adb) in Termux | termux-packages | Apache-2.0 + BSD-2 (recipe) | native | pkg 37.0.0p1, repo pushed 2026-10-18 | **reference** | Same-phone ADB works only under the limits in section 3 |
| Meshtastic / MeshCore Android apps | meshtastic/Meshtastic-Android | GPL-3.0 (API metadata) | Kotlin | Meshtastic v2.8.3-open.1 2026-10-16 | **reject** as a path | Board path (lens 1); see finding 10 |
| Session | getsession.org | not checked | n/a | n/a | **reject** | Needs its own relay network; no LAN mode found (unverified) [S25] |
| Columba (LXMF) | torlando-tech/columba | MPL-2.0 (KB) | Android | n/a | **reference** | Covered by lens 2 |

## 3. Findings that change the plan

1. **What Termux:API offers** [S15]: script list in `termux-api-package/scripts` includes `termux-notification-list`, `termux-notification` (buttons run `dash -c` actions), `termux-sms-*`, `termux-telephony-*`, `termux-clipboard-*`, `termux-share`, `termux-saf-*`, `termux-usb`, `termux-nfc`, `termux-wifi-*`, `termux-open`. There is **no** Bluetooth, accessibility, UI-input or package-install command. Consistent with `plan/cross-layer/devices.md:35-39` (Termux sees no serial, BLE or multicast).
2. **Installing an APK from Termux needs a human.** `termux-open` is `ACTION_VIEW` with a content type [S16], which opens the system installer. The user taps Install, and Termux needs the "install unknown apps" grant (standard Android; unverified in Termux docs). Silent install exists only as `pm install` through ADB shell [S17].
3. **Android developer verification** [S20]: from 2026-09-30, apps installed from *participating stores* in Brazil, Indonesia, Singapore and Thailand on certified devices must be from verified developers; global expansion in 2027. Google's guides page says "ADB workflow and experience stays the same" and sideloading stays possible through an "advanced flow" (details not on the page; press reports a 24-hour wait, secondary). Plan for the user (or adb) to install Briar, SimpleX and Delta Chat from F-Droid or the vendor, never from a 111-hosted APK.
4. **Same-phone ADB.** Wireless debugging needs Android 11+ and a Wi-Fi association [S17]. Pairing is a one-time human step (pairing code and port from Settings); the connection port differs each session [S18]. Termux ships `android-tools` 37.0.0p1 [S15]. Third-party notes (dated 2026-07-01, no test matrix) say the connection drops on screen lock, app switch and reboot and that hotspot-only use is untested [S18]. Secondary reports say Android auto-disables wireless debugging after inactivity or a Wi-Fi change [S24]. Android 17 "ADB Wi-Fi 2.0" turns it off on untrusted networks and re-enables it on user-allowed ones [S17]. Shizuku's guide says to re-start after every reboot [S19]. **Verdict: usable as a user-attended session, not as a standing automated channel.**
5. **What ADB shell can do.** Google documents `am start`, `pm install/grant` and `screencap` [S17]. `input tap/text/keyevent` and `uiautomator dump` are not on that page but are widely used (third-party doc [S18]; unverified from Google). `L1` already drives phones this way from a *host* over USB (`1-bootstrap/README.md:64-68`: `run-as`, `input text`, `dumpsys deviceidle`, phantom-killer settings), so the machinery is known; Layer 6 would add a *same-phone, wireless* variant.
6. **Reading another app's data: no.** Android gives each app its own UID and sandbox [S22]. The only legal read paths are what an app exports and what notifications show (`termux-notification-list`). That needs the user to grant Notification Access, and on Android 13+ "restricted settings" can block that grant for sideloaded APKs until the user uses the app-info menu to allow it (secondary [S23]). The L1 bundle installs Termux:API from GitHub builds, so this applies.
7. **Background limits.** Termux's own README warns Android 12+ kills processes beyond a 32 phantom-process limit and heavy-CPU processes [S15]; L1 already hardens this (`1-bootstrap/README.md:67`). Android 14+ adds a "Disable child process restrictions" toggle (secondary [S26]).
8. **Briar** [S1][S2][S5][S6]. Headless REST (`127.0.0.1:7000`, bearer token in `~/.briar/auth_token`) offers contacts, private messages, forums and blogs, and a WebSocket with message and tick events. `HeadlessModule.kt:99-104` gives Linux only `unixTor`. Contacts are added through a *pending contact* over a Tor hidden service; the README notes nearby (Bluetooth/Wi-Fi) adding exists but is an app feature, not an API call. `LanTcpPlugin` is already in `bramble-core`; a one-line headless change could register it, but the contact would still have to be added over Tor once. The KB entry `briar-mailbox.md` is correct on this point and `briar-bramble.md` has the wrong license string (`GPL-3.0-only`; LICENSE.txt says 3 "or any later version").
9. **Delta Chat in Termux and on a hub** [S7][S8]. Release assets include `deltachat-rpc-server-arm64-v8a-android` and linux `aarch64` and `x86_64` binaries; the protocol is JSON-RPC 2.0 over stdio, so there are no ports to expose. `Config::Bot` marks the account as a bot (core `src/config.rs:329-330`). LAN needs a chatmail relay: Debian 12 with Postfix, Dovecot, Nginx, OpenDKIM; "IP-only" mode is explicitly experimental, uses self-signed TLS and no DKIM, and addresses look like `user@[192.168.1.5]` [S9]. Whether the mobile apps accept such a relay is unverified.
10. **Meshtastic app via ADB.** `Meshtastic-Android` (google flavor, Android 16+) registers an `AppFunctionService` with a `sendMessage` function [S21]. Calling it needs `EXECUTE_APP_FUNCTIONS`, and Google's page says only "a limited number of apps and system agents" can use it (preview). `adb shell cmd app_function ...` is documented as a test route. Interesting as an experiment; the board and `meshmud` path (lens 1) stays the real one. No MeshCore app API was found; the KB's `meshcore.md` lists companion links serial, BLE and TCP on the *board* only.
11. **SimpleX** [S10]. The CLI runs as a local WebSocket server (`simplex-chat -p 5225`); JSON commands with correlation ids; bot profiles use `peerType: "bot"`. `smp-server init -y -l --ip 192.168.1.5` is the documented LAN form. The server doc warns that without a domain, invitation links use the non-browser `simplex:` scheme. The CLI docs mention no Android or Termux build; the binaries are Ubuntu glibc (dynamic linking unverified), so a SimpleX hub must be a Debian/Ubuntu-class box, not a musl-only one.
12. **Jami** [S11]. A LAN-only network needs a bootstrap node (a `dhtnode` on the hub or local peer discovery, which needs UDP broadcast). Forum reports say local accounts show offline when the router has no internet (unresolved; unverified for 2026 builds).
13. **ToS and legal.** None of the open protocols restricts automation. SimpleX's preset servers and the public chatmail relays have operator terms (secondary text [S27]); self-hosted hub servers avoid that. SimpleX and Delta Chat both label bots; use that, never impersonate a person. The legal point that matters is ours: **a bridge makes the hub an endpoint that holds the plaintext of an end-to-end-encrypted app**, and forwarding it onto LoRa publishes it to the channel. Per-contact opt-in and an honest label are required (D23 loop guard and consent apply).
14. **License gate after D22.** SimpleX (AGPL), Briar (GPL), Jami (GPL) stay separate processes (supervised sidecar; their sign-off rows needed). Delta Chat core (MPL-2.0) and Berty (Apache/MIT) and the Matrix Rust crates (Apache-2.0) could link in-binary (MPL secondary-license notice unverified), but the sidecar pattern is cheaper to ship. Nothing here touches Reticulum code.

## 4. Recommendation

**Tiers**

- **Tier (i), bridgeable in v1 through a headless peer on a Linux hub:** SimpleX (offline-LAN capable with a hub SMP server) and Delta Chat (JSON-RPC; LAN capable with a hub chatmail relay, or via a public relay when online). Delta Chat additionally runs inside Termux, so a phone can be a node there. *Online-only member:* Briar (Tor).
- **Tier (ii), reachable only by driving the phone (experimental):** Briar offline (Bluetooth or Wi-Fi in the Android app), Bitchat, Meshtastic/MeshCore apps (AppFunctions or UI), any app with no headless peer. Method: user-paired wireless ADB, `am`/`pm`/`input`/`uiautomator`/`screencap`, notification reading. Always attended, fragile.
- **Tier (iii), no path:** Session, centralized-account messengers (WhatsApp, Telegram, Signal: not researched, need internet), reading any app's private data, silent install without ADB, Bluetooth from Termux.

**Fork A: what v1 promises under "phone apps"**

1. **App-peer adapter shape plus SimpleX and Delta Chat reference adapters (pick).** Same pattern as `meshmud`/presence reader; Linux hub does the work, the phone just runs the normal app. Costs two sidecars; the second is cheap once the first exists.
2. Briar first (the owner's example). Matches the wording but is online-only and JVM-heavy; poor fit for an offline mesh.
3. Delta Chat only. Smallest, best Termux story; weak LAN story (mail stack).
4. Framework only, apps as KB experiments. Lowest effort; "phone apps" becomes a promise with nothing behind it.

**Fork B: ADB and UI driving**

1. **Not in v1; a runbook plus a KB entry for the user-attended wizard (pick).** The user pairs once; the agent only *proposes* typed actions (`install via pm`, `am start <deep link>`, screenshot); `111d` executes after approval. Needs P4 pulled forward in a narrow cut.
2. Opt-in "phone assistant" in v1. More power, but depends on an unstable channel and a new action vocabulary.
3. Agent-owned shell/ADB. Rejected: breaks the "no shell, ever" rule.

**Fork C: Briar offline.** Carry a one-line patch registering the LAN plugin in headless (or ask upstream): helps only after a Tor-based contact add, adds a fork to maintain. Pick: do not patch; document Briar as online-only.

**Pick order if time is short:** SimpleX adapter, then Delta Chat adapter (Termux node as a later gate), Briar online-only last.

## 5. Open questions for the owner

1. Are SimpleX and Delta Chat acceptable as the two named v1 "phone apps" with Briar online-only, given that Briar offline has no headless path?
2. May the hub terminate an app's end-to-end encryption to bridge it (plaintext on the hub, per-contact opt-in, labelled in Messages and on the foreign radio)?
3. The agent has no shell by design. Do you approve moving phone actions (adb, termux-api) into `111d` typed actions with human approval, i.e. a narrow cut of P4 in Layer 6?
4. Is a Debian/Ubuntu-class hub an acceptable floor for the SimpleX and chatmail adapters (glibc binaries; Debian 12 mail stack), with musl-only boards excluded from this category?
5. When the hub has internet, may it use public servers (chatmail relays, SimpleX preset servers) or only self-hosted ones?
6. Do you want a follow-up scout on Matrix (Tuwunel + matrix-rust-sdk, in-binary after D22) and on XMPP/Prosody, which fits LAN federation but was not in this lens?

## 6. Sources

- [S1] Briar headless README: https://github.com/briar/briar/blob/master/briar-headless/README.md (routes, token, pending contacts L190-257)
- [S2] `HeadlessModule.kt`: https://github.com/briar/briar/blob/master/briar-headless/src/main/java/org/briarproject/briar/headless/HeadlessModule.kt (L94-107)
- [S3] Briar LICENSE.txt: https://github.com/briar/briar/blob/master/LICENSE.txt ; tag `release-1.5.22` (commit 2026-10-09): https://github.com/briar/briar/tags
- [S5] Briar Desktop module: https://github.com/briar/briar-desktop/blob/main/briar-desktop/src/main/kotlin/org/briarproject/briar/desktop/DesktopModule.kt (L71-75)
- [S6] `LanTcpPlugin.java`: https://github.com/briar/briar/blob/master/bramble-core/src/main/java/org/briarproject/bramble/plugin/tcp/LanTcpPlugin.java
- [S7] deltachat-rpc-server README: https://github.com/chatmail/core/blob/main/deltachat-rpc-server/README.md
- [S8] chatmail core: https://github.com/chatmail/core/releases/tag/v2.63.0 ; https://github.com/chatmail/core/blob/main/LICENSE ; https://github.com/chatmail/core/blob/main/src/config.rs
- [S9] chatmail relay: https://github.com/chatmail/relay/blob/main/doc/source/iponly.rst ; .../getting_started.rst ; .../overview.rst ; LICENSE (MIT)
- [S10] SimpleX: https://github.com/simplex-chat/simplex-chat/blob/stable/bots/README.md ; .../docs/SERVER.md (L606-633) ; .../docs/CLI.md ; .../LICENSE ; https://github.com/simplex-chat/simplex-chat/releases/tag/v7.0.3 ; https://github.com/simplex-chat/simplexmq/releases
- [S11] Jami: https://docs.jami.net/user/lan-only.html ; https://forum.jami.net/t/jami-without-access-to-global-internet/695 ; https://github.com/savoirfairelinux/jami-daemon ; D-Bus XML `bin/dbus/cx.ring.Ring.ConfigurationManager.xml`
- [S12] Berty: https://github.com/berty/berty/blob/master/README.md ; https://github.com/berty/weshnet
- [S13] Bitchat: https://github.com/permissionlesstech/bitchat ; https://github.com/permissionlesstech/bitchat-android/blob/main/LICENSE.md
- [S14] https://github.com/matrix-construct/tuwunel ; https://github.com/matrix-org/matrix-rust-sdk ; https://github.com/42wim/matterbridge
- [S15] Termux: https://github.com/termux/termux-app (README, LICENSE.md) ; https://github.com/termux/termux-api ; https://github.com/termux/termux-api-package/tree/master/scripts ; https://github.com/termux/termux-packages/blob/master/packages/android-tools/build.sh
- [S16] https://github.com/termux/termux-tools/blob/master/scripts/termux-open.in ; https://github.com/termux/termux-app/wiki/RUN_COMMAND-Intent
- [S17] Android adb: https://developer.android.com/tools/adb ; ADB Wi-Fi 2.0: https://android-developers.googleblog.com/2026/09/wireless-debugging-adb-wifi-2.html
- [S18] Third-party self-connect notes (2026-07-01): https://cdn.jsdelivr.net/gh/ferrumclaudepilgrim/claude-code-android@main/docs/adb-wireless.md
- [S19] Shizuku: https://shizuku.rikka.app/guide/setup/
- [S20] https://developer.android.com/developer-verification ; https://developer.android.com/developer-verification/guides
- [S21] https://developer.android.com/ai/appfunctions ; https://github.com/meshtastic/Meshtastic-Android/blob/main/androidApp/src/google/kotlin/org/meshtastic/app/ai/appfunctions/BaseMeshtasticAppFunctionService.kt
- [S22] https://source.android.com/docs/security/app-sandbox
- [S23] https://www.androidauthority.com/android-13-sideloading-apps-restrictions-3161162 ; https://bayton.org/android/android-13-restricted-permissions/ (secondary)
- [S24] https://discuss.grapheneos.org/d/9054-developer-options-wireless-debugging-resetting-itself-on-reboot ; https://www.androidauthority.com/android-wireless-adb-auto-reconnect-3624945 (seen in search results only; fetch returned 403)
- [S25] https://github.com/oxen-io/oxen-docs-gitbook/blob/master/products-built-on-oxen/session/message-routing.md (secondary)
- [S26] https://www.androidheadlines.com/?p=978046 and https://cosyra.com/guides/termux-signal-9-fix.html (secondary, search results)
- [S27] SimpleX preset-server conditions (mirror, secondary): https://192.3.103.43.sslip.io/privacy

**Not reachable / not done:** `code.briarproject.org` returned 403 (GitHub mirror used; mirror pushed 2026-10-13); the Termux wiki returned an Anubis block; Android Authority returned 403. No phone, Android 14-17 test device, or Termux runtime was available; every device behavior above is from upstream docs or source, not from a run.
