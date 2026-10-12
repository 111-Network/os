---
title: Qwen3 4B
category: model
status: candidate
summary: Compact Apache-licensed LLM with switchable thinking mode, 32K native context.
tags: [llm]
aliases: [Qwen3-4B]
platforms: [linux, android]
upstream: https://huggingface.co/Qwen/Qwen3-4B
license: Apache-2.0
last_verified: 2026-10-12
sources:
  - https://huggingface.co/Qwen/Qwen3-4B
  - https://github.com/QwenLM/Qwen3
  - https://qwen.ai/
relations:
  used_in: ["[[llama-cpp]]"]
---
Compact Apache-licensed LLM with switchable thinking mode, 32K native context.

## What it is

A 4-billion-parameter dense language model from the Qwen3 family, released
under Apache-2.0 with weights on Hugging Face. It supports a hybrid thinking
mode: reasoning traces can be enabled for harder prompts or disabled for
low-latency chat. Native context is 32K tokens, extendable with YaRN to
roughly 131K. Specialized 2507 refresh variants add a 256K context window.
GGUF builds run locally through [[llama-cpp]].

## Use when

A small self-hosted model is needed for drafting, classification, or agent
tool-use on nodes with a few gigabytes of RAM. The permissive license makes
it a safe default for redistribution inside the OS image, and its thinking
toggle lets one model serve both quick answers and deeper planning.

## Do NOT use when

Quality bar exceeds small-model ability: long multi-step reasoning, large
context recall, or precise tool-call reliability still favor bigger models or
cloud APIs. The 4B class is the budget tier, not the capability ceiling.

## Details

Desk research, verified 2026-10-12. Model gate adaptation applies: project
gates plus a mandatory license check, which Apache-2.0 passes cleanly.
Weights are public with a documented model card; the family has a broad
ecosystem of tooling and quantized redistributions. Footprint on ESP32-class
hardware is out of reach; the realistic home is Linux-class nodes and phones
with sufficient RAM. Promotion past candidate requires on-device quality and
latency measurement logged by the OS's own smoke test.

## Evidence

### Alternatives considered

2026-10-12: desk research sweep. Phi-4-mini and Gemma 4-class models were
weighed; Gemma carries a custom license needing review, and Phi-4-mini has a
similar footprint with a different license posture. Devstral 24B and
gpt-oss-20b were recorded as larger-step-up options for hub nodes. Qwen3 4B
stays the compact default for its permissive license and mature tooling
support in GGUF form.

## Links

Runtime pairing: [[llama-cpp]]. Weights: https://huggingface.co/Qwen/Qwen3-4B
