## Details

### Symptom

A case.

### Checks

```toml
schema = 1
severity = "error"

[match]
layer = 0
subject = "kb"
reason = "verify_failed"

[[check]]
id = "failed"
test = { p = "service_reason", id = "@subject", is = "verify_failed" }

[[cause]]
id = "payload_damaged"
when = ["failed"]
action = { action = "human_step", steps = ["Run `sudo 111 update` to fetch the release again."] }
```
