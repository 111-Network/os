//! Layer 9 (§6.2 grammar v1 and the block's layout): T0 stub; filled by row D4.
//!
//! The block under `### Checks`, parsed with the `toml` crate node111 links: `schema` from 1 to
//! `MAX_SCHEMA` (compiled in), every table `deny_unknown_fields`, the twelve predicates tagged on
//! `p` with no fallback variant, the combinators `all`, `any` and `not` (depth at most 3, at most
//! 16 leaves, the block at most 2 KiB), check ids of at most 16 B and cause ids of at most 24 B,
//! and `[match]` inside the compiled vocabulary. An unknown predicate, field, service id or reason
//! code refuses the whole playbook, which never runs in part; so does a cause naming a human-only
//! action, a typed action whose `when` names only `log_contains` checks, or an `@subject` outside
//! the compiled subject table.
//!
//! T0: the names only.
#![allow(dead_code)]

/// A parsed playbook block (§6.2). T0: the name only.
pub struct Playbook;

/// One predicate of grammar v1, or a combinator (§6.2). T0: the name only.
pub struct Predicate;
