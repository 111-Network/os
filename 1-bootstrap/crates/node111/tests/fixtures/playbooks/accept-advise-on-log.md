## Details

### Symptom

A case.

### Checks

```toml
schema = 1

[match]
layer = 5
subject = "meshtastic"
reason = "port_busy"

[[check]]
id = "in_use"
test = { p = "log_contains", id = "meshtastic", literal = "address already in use" }

[[cause]]
id = "other_listener"
when = ["in_use"]
action = { action = "advise", text = "Another program holds the port meshmud needs." }

[[cause]]
id = "look_closer"
when = ["in_use"]
action = { action = "escalate", task = "diagnose" }
```
