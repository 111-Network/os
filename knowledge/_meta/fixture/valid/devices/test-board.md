---
title: Test Board
category: device
status: candidate
summary: Fixture device profile for validator testing only.
tags: [hardware]
platforms: [esp32]
last_verified: 2026-10-12
relations:
  hosted_by: ["[[alpha-project]]"]
---
Fixture device profile for validator testing only.

## What it is

A fictional device profile that exists only to exercise the validator on a known-good entry. It carries the inverse relation that pairs with the project fixture so the relations graph has a complete, warning-free edge pair.

## Use when

Validating that build.py accepts a compliant device entry end to end.

## Do NOT use when

Real hardware guidance is needed; this entry is a fixture and names no real board.

## Details

The body is deliberately long enough to stay clear of the token floor so the fixture produces no budget warnings. It repeats harmless filler sentences about fixture design across several short lines so the shape checks have something real to parse.

## Evidence

### Alternatives considered

2026-10-12: none; this is a synthetic fixture entry, no real alternatives exist.

## Links

Fixture project: [[alpha-project]].
