//! §8.3/§13 — the Cat-1 commissioning ceremony, commissioner side (D14b: owned
//! by the operator's computer running the `111` CLI, `111 identity
//! commission`). Relays never touch mnemonics.
//!
//! Implements (owner: T11):
//!
//! - entropy-verified keygen per §4 on the commissioning device, with the
//!   backup ceremony artifacts (words + single-frame QR) and the
//!   verify-on-write-down gate BEFORE any provisioning (verify-then-provision
//!   order is normative);
//! - provisioning under the §5 carve-out: the derived 64B keyset crosses
//!   exactly one point-to-point local physical channel (USB-UART provisioning
//!   mode or BLE numeric-comparison pairing — never a routed path, never
//!   broadcast radio); the relay-side keyset write is atomic (commit flag),
//!   and re-provisioning is accepted iff no committed keyset exists;
//! - ceremony end: challenge-verify (relay signs a commissioner-chosen nonce
//!   with the provisioned signing key — a misprovisioned relay fails loudly),
//!   then the commissioner MUST wipe its copy of the derived keyset (only the
//!   event is logged, never the material).
//!
//! The relay-side byte-level protocol is a documented stub here — spec'd when
//! Cat-1 firmware lands (§15).
