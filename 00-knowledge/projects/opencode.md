---
title: OpenCode
category: project
status: candidate
summary: Open-source coding agent CLI with terminal TUI and 75+ model providers.
tags: [agent, llm]
platforms: [linux, macos, windows]
upstream: https://github.com/anomalyco/opencode
license: MIT
last_verified: 2026-10-12
sources:
  - https://github.com/anomalyco/opencode
  - https://opencode.ai
---
Open-source coding agent CLI with terminal TUI and 75+ model providers.

## What it is

An open-source coding and agent CLI, reported as of 2026-10 as the
most-starred project in its class with 212,751 GitHub stars. MIT-licensed,
written in TypeScript, last pushed 2026-10-12. It ships a terminal TUI, a
desktop app, and an IDE extension. It is provider-agnostic with 75+
providers, including local models served through Ollama or LM Studio. It
reads LSP context, runs parallel sessions, and separates plan and build
agents. Version 2.x was released in September 2026. It can run headless with
JSONL input/output, which makes it usable as orchestration for an autonomous
maintainer agent.

## Use when

A single coding-agent harness is wanted across terminal, desktop, and IDE
surfaces. Provider choice matters, including local models kept on the
machine. Scripted or headless operation is needed, for example an
orchestration loop driving repository maintenance with structured JSONL
events.

## Do NOT use when

The target node is relay-class or weakest-tier hardware: the TypeScript and
Node runtime is heavy there, so it belongs on hubs, not relays. A stable,
slowly-changing dependency is required without version pinning: the project
moves fast, so versions need pinning. Expected agent quality exceeds what the
local model behind it can deliver — the harness does not raise the model's
ceiling.

## Details

Star count, provider count, and push freshness are reported as of 2026-10
from the 2026-10-12 research sweep, not re-measured locally. The headless
JSONL mode is the property that fits the OS's autonomous maintainer design:
an outer loop can spawn sessions, feed prompts, and parse events without a
terminal. The v2.x line dates from September 2026, so early-adopter drift is
a real risk when pinning. Local-model paths go through OpenAI-compatible
endpoints from Ollama or LM Studio.

## Evidence

### Alternatives considered

2026-10-12: desk research sweep. Other open coding-agent CLIs exist,
including Aider-style git-loop harnesses and editor-centric assistants;
OpenCode leads on star count, MIT license, provider breadth, LSP context,
and headless JSONL operation. Model runtimes such as llama.cpp are
complements underneath it, not alternatives to it.

## Links

Upstream: https://github.com/anomalyco/opencode — Docs: https://opencode.ai
