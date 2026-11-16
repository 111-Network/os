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
id = "none"
test = { p = "all", of = [] }

[[cause]]
id = "k"
when = ["degraded", "none"]
action = { action = "advise", text = "Seen." }
```
