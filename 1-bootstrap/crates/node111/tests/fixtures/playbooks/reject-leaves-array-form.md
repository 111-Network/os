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
id = "wide"
test = ["any", [["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"], ["unclean_exit"]]]

[[cause]]
id = "held_degraded"
when = ["degraded"]
action = { action = "service.restart", service_id = "@subject" }
```
