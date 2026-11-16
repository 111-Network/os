## Details

### Symptom

A case.

### Checks

```toml
schema = 1

[match]
layer = 3
subject = "model"
reason = "child_exit"

[[check]]
id = "held"
[check.test]
p = "all"
of = [
  { p = "service_state", id = "@subject", is = "degraded" },
  { p = "restarts_at_least", id = "@subject", n = 5 },
]

[[check]]
id = "killed"
test = { p = "exit_is", id = "@subject", signal = 9 }

[[check]]
id = "oom"
test = { p = "log_contains", id = "@subject", literal = "failed to allocate" }

[[cause]]
id = "held_degraded"
when = ["held"]
action = { action = "service.restart", service_id = "@subject" }

[[cause]]
id = "out_of_memory"
when = ["killed", "oom"]
action = { action = "advise", text = "The model ran out of memory." }
```
