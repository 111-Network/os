## Details

### Symptom

A case.

### Checks

```toml
schema = 1
check = [["degraded", { p = "service_state", id = "model", is = "degraded" }]]

[match]
layer = 3
subject = "model"
reason = "child_exit"

[[cause]]
id = "held_degraded"
when = ["degraded"]
action = { action = "service.restart", service_id = "@subject" }
```
