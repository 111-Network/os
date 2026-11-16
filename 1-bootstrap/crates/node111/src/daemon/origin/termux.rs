//! Layer 8 (§11.5, §11.1, §0, §15.1 `origin.termux`): T0 stub; filled by row D46 (the pin source
//! beside the constant); row D26 reads the constant.
//!
//! The Termux deb and APK pins of the release manifest are their own pin source under
//! `origin.components` and the operator's "Termux install files" tick (`origin.termux`), offered
//! only by a build whose licence list marks those pins verified. That build-time list is one
//! constant, [`TERMUX_PINS_VERIFIED`]: `false` in v1, so v1 ships the pin source off. Callers read
//! [`pins_verified`]. A debug build also carries a process-wide override,
//! `set_pins_verified_for_test`, so D46's integration test (`tests/datasets_origin.rs`) can mark
//! the pins verified; it is compiled out of a release build (`cfg(debug_assertions)`, the
//! `NODE111_BRIDGE_STUBS` gating), so a shipped `111` has no override and no reader of one.
#![allow(dead_code)]

/// The build-time licence list of the Termux deb and APK pins (§11.5), as one constant: whether
/// this build marks them verified. It stays `false` while `LICENSING.md:64-65` (§3: the Termux,
/// Termux:Boot and Termux:API APKs, and the Termux `.deb` closure) mark both rows unverified; only
/// a verified licence read per pin, recorded there, changes it. Read through [`pins_verified`].
pub const TERMUX_PINS_VERIFIED: bool = false;

/// A debug build's override of [`TERMUX_PINS_VERIFIED`], one for the whole process: 0 none, 1 not
/// verified, 2 verified.
#[cfg(debug_assertions)]
static VERIFIED_FOR_TEST: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Whether the Termux pins count as verified: [`TERMUX_PINS_VERIFIED`], or, in a debug build, the
/// override `set_pins_verified_for_test` set, whichever thread reads it.
pub fn pins_verified() -> bool {
    #[cfg(debug_assertions)]
    {
        let set = VERIFIED_FOR_TEST.load(std::sync::atomic::Ordering::SeqCst);
        if set != 0 {
            return set == 2;
        }
    }
    TERMUX_PINS_VERIFIED
}

/// A test hook, not an API: mark the Termux pins verified (`Some(true)`) or not (`Some(false)`)
/// for the whole process, every thread included (a job's `111d-dataset` thread, the runtime's
/// workers); `None` drops the override. Debug builds only, so a release build has none: a test
/// that calls it is `#[cfg(debug_assertions)]` too, holds one lock with every test of its binary
/// that reads [`pins_verified`], and sets `None` again before it releases that lock.
#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn set_pins_verified_for_test(verified: Option<bool>) {
    let set = match verified {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    };
    VERIFIED_FOR_TEST.store(set, std::sync::atomic::Ordering::SeqCst);
}
