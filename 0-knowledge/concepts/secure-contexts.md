---
title: Secure Contexts
category: concept
summary: Browser rule deciding which APIs work on LAN HTTP versus loopback origins.
tags: [panel, networking]
aliases: [potentially trustworthy origin, localhost secure context]
sources:
  - https://developer.mozilla.org/en-US/docs/Web/Security/Secure_Contexts
  - https://w3c.github.io/webappsec-secure-contexts/
  - https://developer.mozilla.org/en-US/docs/Web/API/Web_Serial_API
  - https://developer.chrome.com/blog/local-network-access
relations:
  relates_to: ["[[meshtastic-web]]", "[[jsqr]]", "[[home-assistant-ingress]]"]
---

Browser rule deciding which APIs work on LAN HTTP versus loopback origins.

## What it is

A secure context is a page the browser treats as safe enough for powerful APIs.
Plain `http://` on a LAN is not one; plain `http://` on loopback is.
The 111 dashboard runs on plain HTTP in v1 (no TLS), so this rule decides which panel features work where.

## Use when

- Deciding whether a hosted panel may rely on Web Serial, Web Bluetooth, WebUSB, or a camera.
- Writing the "open the dashboard on the node itself" copy for a panel that needs a secure context.

## Do NOT use when

- A browser-specific answer is needed for `*.localhost`; support differs (see Details).
- TLS is deployed: with HTTPS on every origin the LAN case changes entirely.

## Details

Exact rule: a framed document is a secure context only if its own origin is potentially trustworthy
and every ancestor frame is too.

| Origin | Secure context |
|---|---|
| `http://127.0.0.1`, `http://[::1]` | yes, unconditionally |
| `http://localhost`, `http://*.localhost` | yes in conforming browsers (loopback-pinned resolution) |
| `http://192.168.x.x`, `http://111.local` | no, and neither are frames inside them |

APIs needing it: Web Serial, Web Bluetooth, WebUSB, `getUserMedia` (QR scanning with [[jsqr]]), and
AudioWorklet (MeshChatX calls). A cross-origin frame also needs the iframe `allow` delegation
(`serial`, `bluetooth`, `usb`); whether `sandbox` blocks that delegation is unverified.
Browser support: Web Serial is Chromium-only on desktop (Chrome for Android reportedly gained it in 148);
Web Bluetooth is Chromium-only, none on iOS. `*.localhost` works in Chromium and Firefox 84+; Safari on macOS
failed through 15.7 and works on macOS 26 per a September 2025 report; iOS unverified.

Consequences in Layer 5: panels that declare `needs_secure_context = true` are framed only on a trustworthy
dashboard origin; otherwise the tab links to the loopback URL. The main listener answers `Host: localhost`
with a 308 to `127.0.0.1`. The Meshtastic panel declares `false`: its HTTP transport works on plain HTTP via the
111d shim, and its Serial and Bluetooth buttons work only on loopback ([[meshtastic-web]]).
Chrome 154 (October 2026) enables HTTPS-by-default but reportedly excludes private addresses at first (Google's announcement is the primary source; press coverage is secondary); whether `.local`
counts as private is unverified. Chrome's Local Network Access prompts (since 142) are why panel-to-device
calls should go through 111d, not cross-origin. Cookies are shared across ports but storage is per origin.

## Evidence

- Rules from MDN and the W3C Secure Contexts specification; the framed-document rule is stated there.
- Browser-version claims marked "reportedly" come from secondary sources and are unverified.
- Panel design rationale: [[home-assistant-ingress]] and the D19 panel decision.

## Links

- [MDN: secure contexts](https://developer.mozilla.org/en-US/docs/Web/Security/Secure_Contexts)
- [W3C specification](https://w3c.github.io/webappsec-secure-contexts/)
- [Chrome Local Network Access](https://developer.chrome.com/blog/local-network-access)
