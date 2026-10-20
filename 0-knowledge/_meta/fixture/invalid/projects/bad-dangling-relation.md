---
title: Bad Dangling Relation
category: project
status: candidate
summary: Fixture entry pointing an edge at nothing.
tags: [mesh]
platforms: [esp32]
upstream: https://example.com/badrel
license: MIT
last_verified: 2026-10-12
relations:
  runs_on: ["[[nowhere-board]]"]
---
Fixture entry pointing an edge at nothing.

## What it is

Intentionally broken: its relations target resolves to no existing entry.

## Use when

Selftest only.

## Do NOT use when

Anything real.

## Details

Invalid fixture.

## Evidence

### Alternatives considered

2026-10-12: none.

## Links

None.
