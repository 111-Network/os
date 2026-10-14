//! `mesh111` — the Layer 5 fabric (spec: `os/messaging/README.md` v1.1).
//!
//! Store-and-forward of Layer 2 sealed packets across whatever links exist:
//! envelope codec, message identity, fragmentation, authenticated receipts,
//! contacts, the home-hub mailbox, the `Link` trait and the encrypted store.
//! The identity keyset never enters this crate: every cryptographic operation
//! goes through the [`sealer::Sealer`] handle the daemon implements over
//! `id111` (spec §1, §18.3).
//!
//! Module ownership follows `os/messaging/TASKS.md` (T0 scaffold; Wave 0 C2/C3;
//! Wave 1 F1). Modules are stubs until their task lands.

#![forbid(unsafe_code)]

pub mod sealer; // T0 — the identity-holder handle (trait only)
pub mod envelope; // C2 — §3 outer `extra`, inner layout, flags, kinds, TLVs
pub mod ids; // C2 — §4 msg_id / msg_seq epochs
pub mod ack; // C2 — §6.1 ACK block (+ ack-sig)
pub mod custody; // C2 — §6.2 custody receipts (+ custody-sig)
pub mod enroll; // C2 — §7.3 enrollment keyed hash
pub mod keys; // C3 — §10.2 KeyProvider seam (Mode S / Mode T)
pub mod store; // C3 — §10 SQLCipher store, tx/rx state, pruning
pub mod blobs; // C3 — §11 BLAKE3/bao blob store (v1: addressing + outboard)
pub mod frag; // F1 — §5 Tier 0/1 fragmentation + reassembly
pub mod link; // F1 — §8 Link trait, tcp-lan adapter, test links
pub mod contacts; // F1 — §7 contacts, cards, requests
pub mod mailbox; // F1 — §9 mailbox client + hub role
pub mod fabric; // F1 — the runtime that ties the above together
