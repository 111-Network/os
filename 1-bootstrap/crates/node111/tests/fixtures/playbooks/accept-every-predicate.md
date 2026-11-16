## Details

### Symptom

A case.

### Checks

```toml
schema = 1
severity = "warning"

[match]
layer = 3
subject = "model"
reason = "child_exit"

[[check]]
id = "state"
test = { p = "service_state", id = "model", is = "backoff" }

[[check]]
id = "reason"
test = { p = "service_reason", id = "model", is = "health_unreachable" }

[[check]]
id = "restarts"
test = { p = "restarts_at_least", id = "model", n = 3 }

[[check]]
id = "count"
test = { p = "incident_count_at_least", n = 2 }

[[check]]
id = "exit"
test = { p = "exit_is", id = "model", code = 1 }

[[check]]
id = "health"
test = { p = "health_is", of = "agent", is = "error" }

[[check]]
id = "file"
test = { p = "file_exists", root = "state", rel = "heal/inbox" }

[[check]]
id = "log"
test = { p = "log_contains", id = "model", literal = "out of memory" }

[[check]]
id = "disk"
test = { p = "disk_free_below", percent = 5 }

[[check]]
id = "unclean"
test = { p = "unclean_exit" }

[[check]]
id = "component"
test = { p = "component_enabled", id = "deltachat", is = false }

[[check]]
id = "release"
test = { p = "release_at_least", calver = "2026.11.16" }

[[cause]]
id = "crash_loop"
when = ["state", "restarts", "count"]
action = { action = "service.restart", service_id = "model" }

[[cause]]
id = "no_disk"
when = ["disk", "file"]
action = { action = "advise", text = "Free some disk space." }

[[cause]]
id = "old_release"
when = ["release", "unclean", "exit", "health", "reason", "component"]
action = { action = "escalate", task = "diagnose" }

[[cause]]
id = "logged"
when = ["log"]
action = { action = "advise", text = "The model logged an allocation failure." }
```
