//! Layer 9 (§5.1 T0; §6.2 the predicates' reads; §7.3 rule 5): T0 stub; filled by row D5.
//!
//! Evaluates a parsed playbook over a `Facts` trait (row D13 implements it over the daemon's state;
//! the fixtures implement it in tests): each check's value, the satisfied cause ids and
//! `checks_satisfied` per mille. Exactly one satisfied cause is the diagnosis; several make the T1
//! enum those causes; none make it the playbook's causes plus `unknown`. A predicate reading what
//! this node lacks evaluates false; a cause satisfied only through `log_contains` yields `advise`.
//! Evaluating makes no request, opens no socket, spawns nothing and keeps no ring of past
//! transitions.
//!
//! T0: the names only.
#![allow(dead_code)]

/// What a predicate reads (§6.2), implemented over the daemon's state in `heal::facts`. T0: the
/// name only.
pub trait Facts {}

/// One playbook's evaluation: each check's value, the satisfied causes and `checks_satisfied`
/// (§5.1). T0: the name only.
pub struct Evaluation;
