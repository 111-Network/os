---
title: jsQR
category: project
status: candidate
summary: Pure-JavaScript QR decoder; camera capture around it needs a secure context.
tags: [panel, identity]
platforms: [linux, android, ios, macos, windows]
upstream: https://github.com/cozmo/jsQR
license: Apache-2.0
last_verified: 2026-10-14
sources:
  - https://github.com/cozmo/jsQR
  - https://developer.mozilla.org/en-US/docs/Web/Security/Secure_Contexts
relations:
  relates_to: ["[[secure-contexts]]"]
---

Pure-JavaScript QR decoder; camera capture around it needs a secure context.

## What it is

jsQR is a TypeScript library that takes raw image pixels and locates, extracts, and parses any QR code in them.
Layer 5 adopts it in the dashboard to read contact cards: the node's genesis announce as a QR, about version 7.
Drawing the QR is separate: the API returns a module bitmap and the UI renders it.

## Use when

- A contact card or `node1…` address must be scanned in the browser, with no server round trip.
- Frames come from a camera on a loopback or secure origin, or from an uploaded image file.

## Do NOT use when

- The dashboard is opened over a LAN IP or `111.local` and a live camera is expected;
  `getUserMedia` is refused there, so only the file-upload fallback works.
- A verified release cadence is required; none was read for this entry.

## Details

Reported by the GitHub API on 2026-10-14: Apache-2.0, about 4,000 stars, created 2015-12-11,
repository pushed 2026-05-15, not archived. A latest release version was not read.
The Layer 4 dependency sign-off and `package.json` entry land with the UI task.
Spec §16 pairs it with `getUserMedia` on loopback or secure origins and a file-upload fallback;
camera scanning on LAN origins is deferred until a secure context exists (see [[secure-contexts]]).
It decodes pixels only; it needs no network and fetches nothing at runtime.

## Evidence

### Alternatives considered

- No other decoder was benchmarked in the Layer 5 research; the review triage (M2) fixed jsQR as the UI decoder.
- Server-side decoding — rejected: it would need the image to leave the browser and a new API surface.
- Browser-native barcode APIs — not evaluated; support varies by browser.
- 2026-10-14 — license taken from the GitHub license endpoint, not from the `LICENSE` file; recheck at sign-off.

## Links

- [Upstream repository](https://github.com/cozmo/jsQR)
- [MDN: secure contexts](https://developer.mozilla.org/en-US/docs/Web/Security/Secure_Contexts)
