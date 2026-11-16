## Details

### Symptom

A case.

### Checks

```toml
schema = 1
cause = [["held_degraded", ["degraded"], { action = "service.restart", service_id = "@subject" }]]

[match]
layer = 3
subject = "model"
reason = "child_exit"

[[check]]
id = "degraded"
test = { p = "service_state", id = "model", is = "degraded" }
```
