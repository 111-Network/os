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
id = "clean"
test = { p = "not", of = ["unclean_exit"] }

[[cause]]
id = "held_degraded"
when = ["degraded", "clean"]
action = { action = "service.restart", service_id = "@subject" }
```
