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
id = "crashed"
test = { p = "all", of = [["unclean_exit"], { p = "incident_count_at_least", n = 2 }] }

[[cause]]
id = "held_degraded"
when = ["degraded", "crashed"]
action = { action = "service.restart", service_id = "@subject" }
```
