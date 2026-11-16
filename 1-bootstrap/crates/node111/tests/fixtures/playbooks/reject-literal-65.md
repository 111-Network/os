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
id = "log"
test = { p = "log_contains", id = "model", literal = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx" }

[[cause]]
id = "k"
when = ["degraded", "log"]
action = { action = "advise", text = "Seen." }
```
