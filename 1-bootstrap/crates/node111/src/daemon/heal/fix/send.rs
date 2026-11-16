//! Layer 9 (§11.3 sending; §11.2 the three bounds; §17.2 `fix_send`): T0 stub; filled by row D36
//! (sending a fix) and row D39 (the vouches sent).
//!
//! Only a human sends: `POST /api/v1/repairs/fixes/{id}/send {to}` on a kept change, `to` the home
//! hub under `home_hub` or, under `contacts`, the home hub or a contact the human picks; each send
//! asks its own `fix_send` consent, showing the exact bytes, the sealed size and "too large for a
//! carrier" above 2,937 B. `111d` builds the body from the incident, the attempt and the verify
//! log, mints `author_seq` from the announce counter, signs it and sends it with `Fabric::send_op`.
//! One `accepted` vouch leaves at an accept of a received fix and one `reverted` vouch when that
//! fix's own loop reverts it; a later Undo sends nothing.
//!
//! T0: the name only.
#![allow(dead_code)]

/// One send of a fix to one recipient (§11.3, §18.4 `FixSend`). T0: the name only.
pub struct FixSend;
