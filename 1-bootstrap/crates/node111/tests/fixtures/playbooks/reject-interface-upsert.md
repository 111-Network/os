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

[[cause]]
id = "k"
when = ["degraded"]
action = { action = "rns.interface.upsert", device_id = "dev-0001", type = "RNodeInterface", params = { frequency = 869525000 } }
```
