---
title: Reticulum
category: project
status: candidate
summary: Cryptography-first mesh networking stack, transport-agnostic, Python reference implementation.
tags: [mesh, networking, offline]
aliases: [Reticulum Network Stack]
platforms: [linux, android, macos, windows]
upstream: https://github.com/markqvist/Reticulum
license: LicenseRef-Reticulum
last_verified: 2026-10-12
sources:
  - https://github.com/markqvist/Reticulum
  - https://reticulum.network/
  - https://markqvist.github.io/Reticulum/manual/whatis.html
relations:
  alternative_to: ["[[meshtastic]]"]
---
Cryptography-first mesh networking stack, transport-agnostic, Python reference implementation.

## What it is

A networking stack where every interface is encrypted and authenticated by
construction. The reference implementation is pure Python with minimal
dependencies, and the stack runs over LoRa, packet radio, serial, TCP, UDP,
and WiFi links interchangeably. Higher-level protocols such as LXMF provide
messaging on top. The project publishes a manual, a mesh architecture, and a
community of compatible implementations in other languages.

## Use when

Encrypted, self-organizing multi-hop networking is needed on Linux-class
hosts, especially where several transports must be bridged under one identity
model. It is a candidate for the encrypted transport layer between nodes that
can run Python or one of the alternative implementations.

## Do NOT use when

The target is ESP32-class hardware: the reference implementation needs Python
and does not run on microcontrollers. Ports such as microReticulum exist but
are separate projects with their own maturity. The non-standard custom license
also needs acceptance review before any core-infra adoption.

## Details

Desk research, verified 2026-10-12. The reference implementation ships a
custom permissive license (recorded here as LicenseRef-Reticulum); the
protocol itself is declared public domain, but the license text is not an OSI
entry, so the candidate gate treats it as a flagged item to resolve before
adoption, never for core infrastructure without a documented exception.
Dependencies are Python 3 plus cryptography and pyserial, keeping the
air-gapped build story simple. Community implementations include a Go port and
a Rust effort, which the 2026 sweep noted as a sign of protocol interest.
Activity is steady with a single lead maintainer, so bus factor is a real
consideration at the testing gate.

## Evidence

### Alternatives considered

2026-10-12: desk research sweep. [[meshtastic]] covers the same mesh job on
microcontrollers with a much larger deployed base but weaker transport
abstraction and no built-in end-to-end identity layer. Yggdrasil was noted as
an encrypted overlay, but it assumes IP underneath rather than multi-hop LoRa.
Reticulum is held as the leading option for Linux-class encrypted mesh links
pending license review and a smoke test.

## Links

Mesh alternative: [[meshtastic]]. Upstream: https://github.com/markqvist/Reticulum
