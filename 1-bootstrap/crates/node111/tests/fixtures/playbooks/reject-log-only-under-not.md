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
id = "quiet"
test = { p = "not", of = { p = "log_contains", id = "model", literal = "ready" } }

[[cause]]
id = "k"
when = ["quiet"]
action = { action = "service.restart", service_id = "model" }
```
