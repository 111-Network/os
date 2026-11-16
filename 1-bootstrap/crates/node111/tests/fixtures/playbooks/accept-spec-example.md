---
title: Repair the model child's exit
category: playbook
status: candidate
summary: An unrequested exit of the model child and its repairs.
tags: [repair, model]
last_verified: 2026-11-16
---
An unrequested exit of the model child and its repairs.

## What it is

The playbook `111d` reads when the model child exits without a request: its checks, the
causes they point at and the one action each cause proposes.

## Use when

The Repairs page shows an incident for the model child's exit.

## Do NOT use when

The model was stopped or restarted on purpose: a requested exit opens no incident.

## Details

### Symptom

The model row leaves `live`; the child restarts, or the supervisor holds it degraded.

### Checks

```toml
schema = 1
severity = "error"

[match]
layer = 3
subject = "model"
reason = "child_exit"

[[check]]
id = "degraded"
test = { p = "service_state", id = "model", is = "degraded" }

[[check]]
id = "oom"
[check.test]
p = "all"
of = [
  { p = "exit_is", id = "model", signal = 9 },
  { p = "log_contains", id = "model", literal = "failed to allocate" },
]

[[cause]]
id = "held_degraded"
when = ["degraded"]
verify = "the model row leaves error and the child stays up"
action = { action = "service.restart", service_id = "@subject" }

[[cause]]
id = "out_of_memory"
when = ["oom"]
action = { action = "advise", text = "The model ran out of memory: a smaller model or more free memory is needed." }
```

### Causes

- `held_degraded`: the supervisor held the child degraded after repeated exits.
- `out_of_memory`: the kernel ended the child while it allocated memory.

### Verify

The model row leaves `error` and the child stays up:

```text
model  live
```

### Undo

A restart cannot be undone; advice changes nothing.

## Evidence

### Alternatives considered

One restart playbook for every child: rejected, the causes differ per child.

## Links

- 9-evolution/README.md §6.2
