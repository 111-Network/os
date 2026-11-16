//! Layer 9 (§10.1, §10.2 the two tags; §11.1 the artefact; §11.2 the `PATCH` payload): T0 stub;
//! filled by row D7.
//!
//! The `fix/v1` body: UTF-8 JSON of at most 4,096 B, signed as exact bytes and never re-serialised,
//! parsed strictly (an unknown field or a duplicate key refuses it; an unknown `schema` is parked
//! "needs a later release"); v1 builds `class: "typed"` with `attachment: null` only. `fix_id` is
//! the BLAKE3-256 of the body. The signed inputs are `111/fix/v1` ‖ `body_len` (u32 LE) ‖ body ‖ 32
//! zero bytes, and `111/fix-vouch/v1` ‖ voucher ‖ `fix_id` ‖ kind ‖ `voucher_seq` (u64 LE), signed
//! through the caller's `sign(tag, msg)` and verified with `verify_strict_tagged` under the author
//! the body names. Known-answer vectors: `9-evolution/verify/vectors/fix-v1.json`.
//!
//! T0: the names only.
#![allow(dead_code)]

/// A `fix/v1` body, kept as its exact bytes (§11.1). T0: the name only.
pub struct FixBody;

/// A vouch: `fix_id`, `kind` (3 `accepted`, 4 `reverted`) and `voucher_seq` (§10.2). T0: the name
/// only.
pub struct Vouch;
