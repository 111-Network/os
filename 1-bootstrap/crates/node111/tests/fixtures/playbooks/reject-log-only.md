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
id = "oom"
test = { p = "log_contains", id = "model", literal = "failed to allocate" }

[[cause]]
id = "k"
when = ["oom"]
action = { action = "service.restart", service_id = "@subject" }
```
