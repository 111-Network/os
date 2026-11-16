## Details

### Symptom

A case.

### Checks

```toml
schema = 1

[match]
layer = 6
subject = "any"
reason = "child_exit"

[[check]]
id = "down"
test = { p = "service_state", id = "@subject", is = "degraded" }

[[cause]]
id = "k"
when = ["down"]
action = { action = "advise", text = "Seen." }
```
