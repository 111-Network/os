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
id = "new"
test = { p = "release_at_least", calver = "v1.2" }

[[cause]]
id = "k"
when = ["degraded", "new"]
action = { action = "advise", text = "Seen." }
```
