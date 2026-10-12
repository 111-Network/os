---
title: llama.cpp
category: project
status: candidate
summary: Portable local LLM inference runtime in plain C/C++.
tags: [llm, runtime]
aliases: [llama cpp]
platforms: [linux, macos, windows, android]
upstream: https://github.com/ggml-org/llama.cpp
license: MIT
last_verified: 2026-10-12
sources:
  - https://github.com/ggml-org/llama.cpp
  - https://github.com/ggml-org/llama.cpp/releases
relations:
  uses: ["[[qwen3-4b]]"]
  used_in: ["[[opencode]]"]
---
Portable local LLM inference runtime in plain C/C++.

## What it is

An inference engine that runs Llama-derived and compatible transformer models
on ordinary CPUs and GPUs with no hard dependencies beyond a C/C++ toolchain.
It ships a server mode with an OpenAI-compatible HTTP API, a CLI chat mode,
and quantization tooling (GGUF) that shrinks models to fit small devices.
Bindings and ports exist for many languages and for Android and iOS.

## Use when

Local inference is needed on hardware the OS targets, from a Linux box down
to a mid-range phone. Its server mode is the default seam for agent harnesses
that expect an OpenAI-style endpoint, and its GGUF ecosystem is where
compact models such as [[qwen3-4b]] are published.

## Do NOT use when

SOTA-quality generation at large scale is required; llama.cpp optimizes for
portability and footprint, not datacenter throughput. It is also the wrong
layer for training or fine-tuning, and exotic architectures may lag behind
upstream releases.

## Details

Desk research, verified 2026-10-12. MIT license, public source under the
ggml-org organization, extremely active history with frequent tagged
releases, and a large multi-org contributor base. The repository provides a
documented network-isolated build (plain cmake) and runs with networking
disabled, which matters for the testing-gate offline check. Packaging in
distros and language ecosystems is broad. Footprint per device class still
needs measurement on the OS's own hardware profiles before promotion past
candidate.

## Evidence

### Alternatives considered

2026-10-12: desk research sweep. Ollama was weighed as the friendliest
packaging, but it is a distribution wrapper rather than the runtime itself.
MLC and ExecuTorch were noted for mobile-class acceleration at the cost of a
heavier toolchain. whisper.cpp and moonshine.cpp were recorded as sibling
projects in the same repository family for audio. llama.cpp stays the default
inference seam for its build simplicity and GGUF ubiquity.

## Links

Model pairing: [[qwen3-4b]]. Consumer: [[opencode]].
Upstream: https://github.com/ggml-org/llama.cpp
