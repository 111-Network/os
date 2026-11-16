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
test = ["service_state", "model", "degraded"]

[[cause]]
id = "held_degraded"
when = ["degraded"]
action = { action = "service.restart", service_id = "@subject" }
```
