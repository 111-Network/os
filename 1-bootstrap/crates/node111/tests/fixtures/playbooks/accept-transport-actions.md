## Details

### Symptom

A case.

### Checks

```toml
schema = 1

[match]
layer = 6
subject = "transport"
reason = "child_exit"

[[check]]
id = "down"
test = { p = "service_state", id = "transport", is = "backoff" }

[[check]]
id = "bridge_on"
test = { p = "component_enabled", id = "lxmf", is = true }

[[cause]]
id = "shared_instance"
when = ["down"]
action = { action = "rns.posture.set", share_instance = true }

[[cause]]
id = "region"
when = ["down"]
action = { action = "radio.region.set", region = "EU_868" }

[[cause]]
id = "bridge"
when = ["down", "bridge_on"]
action = { action = "component.disable", id = "lxmf" }
```
