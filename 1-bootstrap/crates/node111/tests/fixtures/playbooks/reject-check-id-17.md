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
id = "degraded_for_long"
test = { p = "service_state", id = "model", is = "degraded" }

[[cause]]
id = "held_degraded"
when = ["degraded_for_long"]
action = { action = "service.restart", service_id = "@subject" }
```
