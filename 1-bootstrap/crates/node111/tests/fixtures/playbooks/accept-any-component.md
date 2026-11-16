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
id = "repeats"
test = { p = "incident_count_at_least", n = 3 }

[[cause]]
id = "keeps_exiting"
when = ["repeats"]
action = { action = "advise", text = "This component keeps exiting: its log says why." }
```
