## Details

### Symptom

A case.

### Checks

```toml
version = 2
schema = 1

[match]
layer = 3
subject = "model"
reason = "child_exit"

[[check]]
id = "degraded"
test = { p = "service_state", id = "model", is = "degraded" }

[[cause]]
id = "held_degraded"
when = ["degraded"]
action = { action = "service.restart", service_id = "@subject" }
```
