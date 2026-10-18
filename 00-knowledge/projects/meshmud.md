---
title: meshmud
category: project
status: candidate
summary: Meshtastic API multiplexer; single maintainer, v1.0.0, hardware behavior unverified.
tags: [lora, mesh, panel]
platforms: [linux]
upstream: https://github.com/Rikonardo/meshmud
license: MIT
last_verified: 2026-10-14
sources:
  - https://github.com/Rikonardo/meshmud
  - https://pkg.go.dev/github.com/Rikonardo/meshmud
  - https://raw.githubusercontent.com/Rikonardo/meshmud/main/go.mod
relations:
  depends_on: ["[[meshtastic]]"]
---

Meshtastic API multiplexer; single maintainer, v1.0.0, hardware behavior unverified.

## What it is

meshmud is a Go program (about 2.5k lines) that holds the single client slot of a Meshtastic node
and fans it out: one serial or TCP uplink, many TCP downlinks, plus an HTTP downlink.
Layer 5 adopts it so the web panel and the presence reader can both talk to one board.
License: own code MIT (upstream `LICENSE`). Binary licensing: see Details; verify at build.

## Use when

- A Meshtastic node (serial, `tcp://host:4403`, or meshtasticd) must serve several local readers.
- 111d needs one supervised process to own the board's API slot.
- A Wi-Fi board is the phone path: the uplink is the board's TCP address.

## Do NOT use when

- Bus factor matters: there is one maintainer and one tagged release (v1.0.0, 2026-03-20).
- A config replay to a second client is assumed; only a packet-buffer replay is documented.
- Windows or macOS hosts are targets; USB serial pass-through is Linux only here.

## Details

Layer 5 configuration (spec §14): no HTTP downlink; one TCP downlink on `127.0.0.1:11230`;
uplink path validated against a strict pattern; `advertise = false` (mDNS only starts when enabled);
config file mode 0600; supervised child of 111d.
The upstream README example shows a serial uplink, a multi-client TCP downlink on 4403,
and a one-client HTTP downlink on 8000 serving `/api/v1/toradio` and `/api/v1/fromradio` with CORS `*`.
111d's own shim serves those routes instead and reframes bodies onto per-session meshmud TCP connections.

License note: meshmud's own code is MIT and its Go closure is 14 permissive modules. Read on 2026-10-14:
`go.mod` lists no Meshtastic or protobuf module and `internal/common/protobuf` is a hand-written wire parser,
so the CI-built binary is conveyed under MIT with its notices; the build's verdict step (module list plus a
`*.pb.go` / import scan) flips it to GPL-3.0 only if a Meshtastic protobuf module ever enters the closure.
Build is CI-only from an upstream tag and tarball sha256, never copied into the `os/` tree.

Unverified: second-client config replay, a static musl or Android build, and DTR/RTS handling when the
serial port opens (no handling found; MeshMonitor's serial bridge disables HUPCL explicitly, so a reset risk is real).
Termux can run only the TCP-to-board uplink; USB serial there is not possible.

## Evidence

### Alternatives considered

- [[meshmonitor]] Virtual Node — a TCP-only proxy on port 4404; the official web client cannot connect
  through it, so it solves presence but not the panel.
- A shim of our own on the node HTTP API — more custom code; 111d still builds the thin per-session shim.
- meshtastic-websocket-proxy — rejected: the web client has no WebSocket transport; license unverified.
- A socat serial bridge (BSD-3-Clause shell script image) — no fan-out.
- 2026-10-14 — license read from upstream; `go.mod` and source tree inspected; hardware behavior not tested.

## Links

- [Upstream repository](https://github.com/Rikonardo/meshmud)
- [Go package page](https://pkg.go.dev/github.com/Rikonardo/meshmud)
- Node it fronts: [[meshtastic]]; client it serves: [[meshtastic-web]]; reader: [[meshtastic-python]]
