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
id = "full"
test = { p = "service_reason", id = "model", is = "store_full" }

[[cause]]
id = "k"
when = ["degraded", "full"]
action = { action = "advise", text = "Seen." }
```
