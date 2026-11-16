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
id = "count"
test = { p = "incident_count_at_least", n = 0 }

[[cause]]
id = "k"
when = ["degraded", "count"]
action = { action = "advise", text = "Seen." }
```
