---
title: llama.cpp
category: project
status: candidate
summary: Low-level portable C/C++ GGUF inference engine for CPU-first local inference.
tags: [llm, runtime, offline]
aliases: [llama cpp]
platforms: [linux, macos, windows, android]
upstream: https://github.com/ggml-org/llama.cpp
license: MIT
last_verified: 2026-10-12
sources:
  - https://github.com/ggml-org/llama.cpp
---
Low-level portable C/C++ GGUF inference engine for CPU-first local inference.

## What it is

A portable C/C++ inference engine for GGUF models, MIT-licensed, embeddable
directly as a library. CPU-first, with GPU offload via CUDA, Metal, Vulkan,
and SYCL. Reported as of 2026-10: 130,866 GitHub stars, with daily pushes.

## Use when

Running 4-8B language models on weak nodes, from Raspberry Pi-class boards
up to servers, where aggressive quantization (4-bit and below) keeps models
viable. It is the de facto backend behind most local-LLM tooling, so adopting
it rides an existing ecosystem. Pairs with compact models such as [[qwen3-4b]]
and backs agent harnesses such as [[opencode]].

## Do NOT use when

A turnkey product is needed: llama.cpp is low-level, so serving and model
management are built by the integrator. On phones, Vulkan and NPU paths are
less turnkey than ExecuTorch. Giant MoE models are impractical on it.

## Details

Desk research, verified 2026-10-12. MIT license; canonical repository under
the ggml-org organization. Scale and activity reported as of 2026-10:
130,866 stars and daily pushes. Runs from Raspberry Pi-class hardware up to
servers; offload backends cover CUDA, Metal, Vulkan, and SYCL. Quantization
at 4-bit and below is what makes 4-8B models viable on weak nodes, matching
the OS's small-model target. Embeddable as a library, which fits the
adopt-vs-build rule of wrapping a proven seam rather than writing inference
plumbing.

## Evidence

### Alternatives considered

2026-10-12: desk research sweep. ExecuTorch offers more turnkey Vulkan/NPU
support on phones, at the cost of a heavier and less portable toolchain.
llama.cpp stays the default inference seam for its C/C++ portability,
library embeddability, and GGUF ecosystem ubiquity; serving and model
management remain integrator-built by design.

## Links

Upstream: https://github.com/ggml-org/llama.cpp
Model pairing: [[qwen3-4b]]. Consumer: [[opencode]].
