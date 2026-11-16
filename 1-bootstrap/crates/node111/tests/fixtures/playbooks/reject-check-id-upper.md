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
id = "Degraded"
test = { p = "service_state", id = "model", is = "degraded" }

[[cause]]
id = "held_degraded"
when = ["Degraded"]
action = { action = "service.restart", service_id = "@subject" }
```
