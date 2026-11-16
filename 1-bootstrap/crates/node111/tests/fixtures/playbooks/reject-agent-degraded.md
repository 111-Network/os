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
id = "agent"
test = { p = "health_is", of = "agent", is = "degraded" }

[[cause]]
id = "k"
when = ["degraded", "agent"]
action = { action = "advise", text = "Seen." }
```
