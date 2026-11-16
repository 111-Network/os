## Details

### Symptom

A case.

### Checks

```toml
schema = 1

[match]
layer = 1
subject = "111d"
reason = "witness_absent"

[[check]]
id = "open"
test = { p = "incident_count_at_least", n = 1 }

[[cause]]
id = "no_witness"
when = ["open"]

[cause.action]
action = "human_step"
steps = [
  "On Linux: re-run `sudo 111 install` to add the exit witness",
  "On a phone: on your computer, connected to this phone, run `111 install --phone` again",
]
```
