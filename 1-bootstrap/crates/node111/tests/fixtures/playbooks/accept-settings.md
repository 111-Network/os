## Details

### Symptom

A case.

### Checks

```toml
schema = 1

[match]
layer = 5
subject = "fabric"
reason = "store_full"

[[check]]
id = "full"
test = { p = "service_reason", id = "fabric", is = "store_full" }

[[check]]
id = "low_disk"
test = { p = "disk_free_below", percent = 10 }

[[cause]]
id = "custody_quota"
when = ["full"]
verify = "the fabric row leaves error"
action = { action = "settings.set", key = "fabric.quota.custody", value = 1048576 }

[[cause]]
id = "hub_off"
when = ["full", "low_disk"]
action = { action = "settings.set", key = "fabric.hub_role", value = false }
```
