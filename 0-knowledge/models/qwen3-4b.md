---
title: Qwen3 4B
category: model
status: candidate
summary: Apache-2.0 4B-parameter LLM family default for local reasoning, coding, and agents.
tags: [llm, offline]
platforms: [linux, android]
upstream: "https://huggingface.co/Qwen"
license: Apache-2.0
last_verified: 2026-10-12
sources: ["https://huggingface.co/Qwen", "https://github.com/ggml-org/llama.cpp"]
---

Apache-2.0 4B-parameter LLM family default for local reasoning, coding, and agents.

## What it is

Qwen3 4B is a 4-billion-parameter open weights language model from Alibaba,
released under Apache-2.0.
It is the repeatedly cited default all-rounder for reasoning, coding, and
multilingual workloads in 2026.
Reported as of 2026-10, it needs roughly 3-4 GB of VRAM at Q4 quantization
and runs on Pi 5-class hardware and old laptops.

## Use when

- A local model must run offline on modest hardware under a permissive license.
- Relay-class and hub-class nodes want one model family across size tiers.
- Tool-calling and agent behavior matter more than frontier benchmark peaks.
- The serving path is GGUF quantization through [[llama-cpp]].

## Do NOT use when

- Long-horizon agentic tasks demand frontier closed-model quality;
  the gap is meaningful at this size.
- Deployer policy excludes Chinese-origin model families.
- A node cannot spare roughly 3-4 GB of VRAM, or equivalent RAM, at Q4.

## Details

The Qwen3 family scales from 0.6B to 32B dense parameters plus a 30B-A3B
MoE variant, so one family spans relay-class to hub-class nodes.
Community GGUF quantization support is huge, and llama.cpp is the de facto
serving runtime for that ecosystem.
Reported as of 2026-10: strong tool-calling and agent behavior for its size
class, with a large catalog of community quantizations.
Platforms noted as of 2026-10: linux and android.

## Evidence

- 2026-10-12 — Desk research sweep; no first-party field testing yet.

### Alternatives considered

- Smaller Qwen3 dense variants (0.6B-1.7B) for relay-class nodes;
  weaker at agent tasks than the 4B point.
- The 30B-A3B MoE variant for hub-class nodes; heavier per-node footprint.
- Frontier closed models; higher quality ceiling but incompatible with
  offline operation and permissive licensing goals.

## Links

- Upstream: https://huggingface.co/Qwen
- Serving runtime: [[llama-cpp]]
- Provenance sources are listed in the frontmatter.
