//! Layer 9 (§12.6 the APK refresh; §3.4 the Layer 1 row): T0 stub; filled by row D50.
//!
//! After each verified manifest check, `111d` compares the manifest's `apk.*` pins with
//! `state/apk.json` (`{"schema": 1, "apk": {<package>: <sha256>}}`, pushed by the host provisioner
//! after the APK step of `111 install --phone`): a difference opens `(1, 111d, apk_outdated)` with
//! its playbook's one `human_step`, a match closes it and an absent file opens nothing. It never
//! implies an automatic install.
//!
//! T0: the name only; nothing is read.
#![allow(dead_code)]

/// `state/apk.json` (§12.6). T0: the name only.
pub struct ApkPins;
