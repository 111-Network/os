//! Layer 9 (§11.4 steps 1 to 5 and 8; §4.1 rule 5): T0 stub; filled by row D37 (the consumer and
//! the trust checks) and row D39 (the vouches received).
//!
//! Layer 9's consumer for `PATCH` (0x13), registered on Layer 8's consumer registry: it returns
//! only after its own durable J9 write and is idempotent by `msg_id`. The body is parsed strictly
//! and its signature verified; the frame's authenticated sender must be the body's `author`, the
//! author in the set `heal.fixes.receive` names and `author_seq` above the author's `high_water`;
//! anything else is dropped and counted, a stranger's fix leaving no record. Another class or an
//! attachment is parked "needs a later release", a mismatched `applies_to` reads "does not apply
//! here", and at most 16 fixes queue per author. A vouch counts only from a node this node sent
//! that fix to. While `heal` is `full`, or with the store closed, a fix is dropped and counted.
//!
//! T0: the name only.
#![allow(dead_code)]

/// The 0x13 consumer (§11.4 step 1). T0: the name only.
pub struct FixConsumer;
