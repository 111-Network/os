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
id = "deep"
test = { p = "all", of = [{ p = "any", of = [{ p = "unclean_exit" }, { p = "incident_count_at_least", n = 1 }] }, { p = "not", of = { p = "unclean_exit" } }] }

[[check]]
id = "wide"
test = { p = "all", of = [{ p = "unclean_exit" }, { p = "unclean_exit" }, { p = "unclean_exit" }, { p = "unclean_exit" }, { p = "unclean_exit" }, { p = "unclean_exit" }, { p = "unclean_exit" }, { p = "unclean_exit" }, { p = "unclean_exit" }, { p = "unclean_exit" }, { p = "unclean_exit" }, { p = "unclean_exit" }, { p = "unclean_exit" }] }

[[cause]]
id = "both"
when = ["deep", "wide"]
action = { action = "advise", text = "Seen." }
```
