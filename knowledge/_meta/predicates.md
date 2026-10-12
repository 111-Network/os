# Predicates — controlled relations vocabulary (normative)

`relations:` maps a predicate to a flow list of double-quoted `[[wikilink]]` targets. Only the predicates below are legal. The subject is the entry carrying the block; the object is the target entry. Tags are not edges, and a relationship stated only in prose does not exist.

## Vocabulary

| Predicate | Inverse | Subject → object meaning |
|---|---|---|
| `implements` | `implemented_by` | subject implements the object protocol/spec |
| `implemented_by` | `implements` | object implements the subject (inverse of `implements`) |
| `runs_on` | `hosted_by` | subject runs on the object device |
| `hosted_by` | `runs_on` | object runs on the subject (inverse of `runs_on`) |
| `depends_on` | `required_by` | subject cannot work without the object |
| `required_by` | `depends_on` | object depends on the subject (inverse of `depends_on`) |
| `used_in` | `uses` | subject is used inside the object |
| `uses` | `used_in` | object uses the subject (inverse of `used_in`) |
| `supersedes` | `superseded_by` | subject replaces the object |
| `superseded_by` | `supersedes` | object supersedes the subject (inverse of `supersedes`) |
| `alternative_to` | symmetric | subject and object are interchangeable options for the same job; one direction satisfies both |
| `relates_to` | symmetric | catch-all association; last resort — near-zero signal |

Direction conventions: a recipe carries `depends_on` (its project) and `runs_on` (its device), mirroring its filename; a project carries `implements` toward a protocol and `runs_on` toward a device. Inverse edges live on the target entry.

## Edge lint rules

| Rule | Severity |
|---|---|
| Unknown predicate | error |
| Dangling target (edges never dangle; `_meta/planned.md` excuses prose links only, never relations) | error |
| Self-edge | error |
| `supersedes` cycle | error |
| Edge to an entry with `status: hold` (hold is forbidden for new nodes; the linter flags every edge to hold) | error |
| Missing inverse on an asymmetric predicate | warning (expected to be noisy during skeleton) |
| `platforms` token matching an existing device slug absent from `runs_on` — `platforms` is search shorthand; `runs_on`/`hosted_by` are the machine edges | warning |

Inverse check: for edge A `-p→` B whose predicate `p` has inverse `q`, expect B `-q→` A; symmetric predicates (`alternative_to`, `relates_to`) satisfy their own inverse, so one direction satisfies both and no warning fires for the missing second direction.
