## Details

### Symptom

A case.

### Checks

```toml
schema = 1

[match]
layer = 1
subject = "111d"
reason = "unclean_exit"

[[check]]
id = "open"
test = { p = "incident_count_at_least", n = 1 }

[[cause]]
id = "k"
when = ["open"]
action = { action = "service.restart", service_id = "@subject" }
```
