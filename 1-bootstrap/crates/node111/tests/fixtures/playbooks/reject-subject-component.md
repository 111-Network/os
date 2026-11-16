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
id = "open"
test = { p = "incident_count_at_least", n = 1 }

[[cause]]
id = "k"
when = ["open"]
action = { action = "component.disable", id = "@subject" }
```
