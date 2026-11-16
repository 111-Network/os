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
id = "exit"
test = { p = "exit_is", id = "model", code = 1, signal = 9 }

[[cause]]
id = "k"
when = ["degraded", "exit"]
action = { action = "advise", text = "Seen." }
```
