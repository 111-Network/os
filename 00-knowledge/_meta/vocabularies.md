# Vocabularies — controlled lists (normative)

`build.py` reads the first token of each `- ` line under `## Platforms` and `## Tags`; keep one token per line so the parser stays trivial.

## Platforms

Family-granularity shorthand for the `platforms:` frontmatter field — lowercase, one token per OS/architecture family. Adding a platform requires editing this file FIRST: an unknown platform token is a lint error (schema F12).

- esp32 — Espressif ESP32 family (including S3 variants)
- nrf52 — Nordic nRF52 family
- rp2040 — Raspberry Pi RP2040
- android — Android phones and tablets
- ios — iPhone and iPad
- linux — Linux, any distro or architecture
- macos — macOS
- windows — Windows

## Tags

Filtering vocabulary for the `tags:` frontmatter field — filtering only, never relations. New tags are allowed without editing this file, but the linter warns so they get reviewed (schema F11); add accepted tags here.

- mesh — mesh networking topologies and routing
- lora — LoRa/LoRaWAN radio
- networking — general networking
- identity — cryptographic identity and keys
- sync — data and message synchronization
- crdt — CRDTs and mergeable data structures
- llm — language models
- runtime — model and agent runtimes
- agent — agent harnesses and tooling
- storage — databases and persistence
- hardware — devices and boards
- offline — offline-first operation
- messaging — message transport, delivery states, and message stores
- panel — third-party web UIs hosted inside the dashboard
- dtn — delay-tolerant networking and store-and-forward custody
- bridge — relaying text between 111 and a foreign network (Layer 6)
- firmware — device firmware images and their flashing tools
- test-fixture — tools used only to test, never shipped
