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
id = "degraded"
test = { p = "service_state", id = "model", is = "degraded" }

[[check]]
id = "loop"
test = { p = "restarts_at_least", id = "model", n = 3, within_events = 10 }

[[cause]]
id = "k"
when = ["degraded", "loop"]
action = { action = "advise", text = "Seen." }
```
