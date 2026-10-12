---
title: OpenCode
category: project
status: candidate
summary: Terminal-native open-source AI coding agent, provider-agnostic, local-model capable.
tags: [agent]
aliases: [opencode]
platforms: [linux, macos, windows]
upstream: https://github.com/anomalyco/opencode
license: MIT
last_verified: 2026-10-12
sources:
  - https://github.com/anomalyco/opencode
  - https://opencode.ai/
relations:
  uses: ["[[llama-cpp]]"]
---
Terminal-native open-source AI coding agent, provider-agnostic, local-model capable.

## What it is

An MIT-licensed coding agent that runs as a terminal UI, reading a codebase,
editing files, and executing commands in an agentic loop. It is
provider-agnostic through a model catalog and speaks to local runtimes via
OpenAI-compatible endpoints, so a self-hosted stack such as
[[llama-cpp]] can serve as its backend. LSP integration supplies code
diagnostics, and MCP support extends its tool surface. The repository moved
from the sst organization to anomalyco in 2026.

## Use when

A scriptable, self-hosted coding agent is needed that does not hard-wire one
vendor's API. It is a candidate harness for the OS's own build agents,
especially where runs must stay on local infrastructure with credentials
held on the machine.

## Do NOT use when

A managed IDE-integrated assistant with cloud-only models is acceptable
instead; OpenCode's value is control and portability, which costs setup
effort. It is also not a model itself, so model quality depends on the
configured backend.

## Details

Desk research, verified 2026-10-12. MIT license, very active public history,
large multi-contributor base, and a documented build path. The project
supports air-gapped deployment with local providers, and credentials stay on
the user's machine, which aligns with the OS's offline-first posture. The
harness is a TypeScript rewrite of an earlier Go version; that history is a
maturity signal worth re-checking at the testing gate. Telemetry defaults
need verification in a local build during the smoke test.

## Evidence

### Alternatives considered

2026-10-12: desk research sweep. Goose and Aider-pattern harnesses were
weighed; Goose overlaps in provider-agnosticism, while Aider-style git-loop
agents are editor-centric. pi-mono-class loops were noted for minimal
runtimes. OpenCode stays the leading harness candidate for its MIT license,
terminal-first design, and native AGENTS.md ecosystem alignment.

## Links

Local backend: [[llama-cpp]]. Upstream: https://github.com/anomalyco/opencode
