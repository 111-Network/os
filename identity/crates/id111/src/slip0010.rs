//! §2 — SLIP-0010 derivation at the pinned identity paths.
//!
//! Implements (owner: T2): SLIP-0010 hardened-only Ed25519 derivation
//! (HMAC-SHA512 keyed `"ed25519 seed"`) from the 64-byte BIP-39 seed:
//!
//! - `m/111'/0'/0'` → the Ed25519 signing key (key half = first 32 bytes of
//!   the SLIP-0010 output, used directly as the RFC 8032 private seed; the
//!   chain-code half is discarded);
//! - `m/111'/0'/1'` → the X25519 ECDH key (32-byte key half used as the
//!   private scalar, clamped per RFC 7748 at point of use).
//!
//! All components hardened; one account, fixed key indices. This is NOT
//! BIP32-Ed25519/Cardano — the two are incompatible and restore different
//! keys from the same words (§2, normative). Test vectors: §14 — the
//! published SLIP-0010 ed25519 test vectors 1 and 2 plus the two pinned
//! project paths (printed by the tests for freezing by the vector task).

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha512;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// HMAC-SHA512 as keyed by SLIP-0010.
type HmacSha512 = Hmac<Sha512>;

/// The SLIP-0010 ed25519 master key: HMAC-SHA512 keyed with the literal
/// string `"ed25519 seed"` (§2, normative).
const MASTER_HMAC_KEY: &[u8] = b"ed25519 seed";

/// The hardened bit (2³¹). All §2 path components are hardened; SLIP-0010
/// ed25519 derivation is hardened-only.
const HARDENED: u32 = 0x8000_0000;

/// §2 pinned path for the Ed25519 signing key: `m/111'/0'/0'`.
/// Indices are given WITHOUT the hardened bit; hardening is applied
/// internally (SLIP-0010 ed25519 is hardened-only).
pub const SIGNING_PATH: [u32; 3] = [111, 0, 0];

/// §2 pinned path for the X25519 ECDH key: `m/111'/0'/1'`.
pub const ECDH_PATH: [u32; 3] = [111, 0, 1];

/// Derivation input error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slip0010Error {
    /// An index with the hardened bit already set was supplied. This API takes
    /// raw indices and hardens internally (SLIP-0010 ed25519 is hardened-only,
    /// §2), so a pre-hardened index is a caller bug.
    IndexOutOfRange(u32),
}

impl std::fmt::Display for Slip0010Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Slip0010Error::IndexOutOfRange(i) => write!(
                f,
                "§2 SLIP-0010: index {i:#x} already carries the hardened bit; pass raw indices"
            ),
        }
    }
}

impl std::error::Error for Slip0010Error {}

/// One SLIP-0010 node: 32-byte key half ‖ 32-byte chain half. The chain half
/// is internal to derivation and discarded at the pinned leaves (§2); both
/// halves zeroize on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
struct Node {
    key: [u8; 32],
    chain: [u8; 32],
}

fn hmac_sha512(key: &[u8], data_parts: &[&[u8]]) -> [u8; 64] {
    let mut mac = HmacSha512::new_from_slice(key).expect("HMAC accepts keys of any length");
    for part in data_parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

/// SLIP-0010 master node from the seed: `I = HMAC-SHA512("ed25519 seed", seed)`,
/// key half = I[0..32], chain half = I[32..64].
fn master_node(seed: &[u8]) -> Node {
    let i = hmac_sha512(MASTER_HMAC_KEY, &[seed]);
    let mut key = [0u8; 32];
    let mut chain = [0u8; 32];
    key.copy_from_slice(&i[..32]);
    chain.copy_from_slice(&i[32..]);
    Node { key, chain }
}

/// SLIP-0010 hardened child: `I = HMAC-SHA512(parent_chain, 0x00 ‖ parent_key
/// ‖ ser32(2³¹ + index))`. Ed25519 derivation is hardened-only (§2) — there is
/// no public derivation here by construction. `index` is the RAW index
/// (< 2³¹); the hardened bit is applied inside.
fn hardened_child(parent: &Node, index: u32) -> Node {
    debug_assert!(index < HARDENED);
    let ser = (index | HARDENED).to_be_bytes();
    let i = hmac_sha512(&parent.chain, &[&[0x00], &parent.key, &ser]);
    let mut key = [0u8; 32];
    let mut chain = [0u8; 32];
    key.copy_from_slice(&i[..32]);
    chain.copy_from_slice(&i[32..]);
    Node { key, chain }
}

/// Derive the node at `path` (raw indices, hardened internally) from the
/// 64-byte BIP-39 seed. Private: callers take the key half only (§2 — the
/// chain half is discarded); tests assert both halves against the published
/// SLIP-0010 vectors.
fn derive_node(seed: &[u8], path: &[u32]) -> Result<Node, Slip0010Error> {
    for &i in path {
        if i >= HARDENED {
            return Err(Slip0010Error::IndexOutOfRange(i));
        }
    }
    let mut node = master_node(seed);
    for &i in path {
        node = hardened_child(&node, i);
    }
    Ok(node)
}

/// Derive the 32-byte key half at `path` from the 64-byte BIP-39 seed; the
/// chain half is discarded (zeroized) per §2.
pub fn derive_key_half(seed: &[u8], path: &[u32]) -> Result<Zeroizing<[u8; 32]>, Slip0010Error> {
    let node = derive_node(seed, path)?;
    Ok(Zeroizing::new(node.key))
}

/// §2 — the Ed25519 signing seed at the pinned path `m/111'/0'/0'`.
pub fn derive_signing_seed(seed: &[u8]) -> Zeroizing<[u8; 32]> {
    derive_key_half(seed, &SIGNING_PATH).expect("pinned §2 path indices are raw and valid")
}

/// §2 — the X25519 ECDH private scalar at the pinned path `m/111'/0'/1'`
/// (returned raw; clamped per RFC 7748 at point of use).
pub fn derive_ecdh_scalar(seed: &[u8]) -> Zeroizing<[u8; 32]> {
    derive_key_half(seed, &ECDH_PATH).expect("pinned §2 path indices are raw and valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn hex32(s: &str) -> [u8; 32] {
        hex(s).try_into().unwrap()
    }

    fn hex64(s: &str) -> [u8; 64] {
        hex(s).try_into().unwrap()
    }

    fn to_hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// The published SLIP-0010 ed25519 test vectors (SatoshiLabs slip-0010),
    /// both seeds, every chain — key half AND chain half byte-exact.
    #[test]
    fn slip0010_published_vectors() {
        // Test vector 1, seed 000102030405060708090a0b0c0d0e0f.
        let seed1 = hex("000102030405060708090a0b0c0d0e0f");
        let v1: &[(&[u32], &str, &str)] = &[
            (
                &[],
                "2b4be7f19ee27bbf30c667b642d5f4aa69fd169872f8fc3059c08ebae2eb19e7",
                "90046a93de5380a72b5e45010748567d5ea02bbf6522f979e05c0d8d8ca9fffb",
            ),
            (
                &[0],
                "68e0fe46dfb67e368c75379acec591dad19df3cde26e63b93a8e704f1dade7a3",
                "8b59aa11380b624e81507a27fedda59fea6d0b779a778918a2fd3590e16e9c69",
            ),
            (
                &[0, 1],
                "b1d0bad404bf35da785a64ca1ac54b2617211d2777696fbffaf208f746ae84f2",
                "a320425f77d1b5c2505a6b1b27382b37368ee640e3557c315416801243552f14",
            ),
            (
                &[0, 1, 2],
                "92a5b23c0b8a99e37d07df3fb9966917f5d06e02ddbd909c7e184371463e9fc9",
                "2e69929e00b5ab250f49c3fb1c12f252de4fed2c1db88387094a0f8c4c9ccd6c",
            ),
            (
                &[0, 1, 2, 2],
                "30d1dc7e5fc04c31219ab25a27ae00b50f6fd66622f6e9c913253d6511d1e662",
                "8f6d87f93d750e0efccda017d662a1b31a266e4a6f5993b15f5c1f07f74dd5cc",
            ),
            (
                &[0, 1, 2, 2, 1000000000],
                "8f94d394a8e8fd6b1bc2f3f49f5c47e385281d5c17e65324b0f62483e37e8793",
                "68789923a0cac2cd5a29172a475fe9e0fb14cd6adb5ad98a3fa70333e7afa230",
            ),
        ];
        for (path, want_key, want_chain) in v1 {
            let node = derive_node(&seed1, path).unwrap();
            assert_eq!(node.key, hex32(want_key), "key half, path {path:?}");
            assert_eq!(node.chain, hex32(want_chain), "chain half, path {path:?}");
            // The public API returns exactly the key half.
            assert_eq!(&derive_key_half(&seed1, path).unwrap()[..], &node.key[..]);
        }

        // Test vector 2, seed fffc…4542.
        let seed2 = hex(
            "fffcf9f6f3f0edeae7e4e1dedbd8d5d2cfccc9c6c3c0bdbab7b4b1aeaba8a5a2\
             9f9c999693908d8a8784817e7b7875726f6c696663605d5a5754514e4b484542"
                .replace(' ', "")
                .as_str(),
        );
        let v2: &[(&[u32], &str, &str)] = &[
            (
                &[],
                "171cb88b1b3c1db25add599712e36245d75bc65a1a5c9e18d76f9f2b1eab4012",
                "ef70a74db9c3a5af931b5fe73ed8e1a53464133654fd55e7a66f8570b8e33c3b",
            ),
            (
                &[0],
                "1559eb2bbec5790b0c65d8693e4d0875b1747f4970ae8b650486ed7470845635",
                "0b78a3226f915c082bf118f83618a618ab6dec793752624cbeb622acb562862d",
            ),
            (
                &[0, 2147483647],
                "ea4f5bfe8694d8bb74b7b59404632fd5968b774ed545e810de9c32a4fb4192f4",
                "138f0b2551bcafeca6ff2aa88ba8ed0ed8de070841f0c4ef0165df8181eaad7f",
            ),
            (
                &[0, 2147483647, 1],
                "3757c7577170179c7868353ada796c839135b3d30554bbb74a4b1e4a5a58505c",
                "73bd9fff1cfbde33a1b846c27085f711c0fe2d66fd32e139d3ebc28e5a4a6b90",
            ),
            (
                &[0, 2147483647, 1, 2147483646],
                "5837736c89570de861ebc173b1086da4f505d4adb387c6a1b1342d5e4ac9ec72",
                "0902fe8a29f9140480a00ef244bd183e8a13288e4412d8389d140aac1794825a",
            ),
            (
                &[0, 2147483647, 1, 2147483646, 2],
                "551d333177df541ad876a60ea71f00447931c0a9da16f227c11ea080d7391b8d",
                "5d70af781f3a37b829f0d060924d5e960bdc02e85423494afc0b1a41bbe196d4",
            ),
        ];
        for (path, want_key, want_chain) in v2 {
            let node = derive_node(&seed2, path).unwrap();
            assert_eq!(node.key, hex32(want_key), "key half, path {path:?}");
            assert_eq!(node.chain, hex32(want_chain), "chain half, path {path:?}");
        }
    }

    /// §14 — the pinned project-path vectors (`m/111'/0'/0'`, `m/111'/0'/1'`)
    /// over the canonical-mnemonic BIP-39 seed. PRINTED for freezing by the
    /// test-vector task (run with --nocapture); the printed values are asserted
    /// below once frozen.
    #[test]
    fn slip0010_project_path_vectors() {
        // BIP-39 seed of "abandon"×11 + "about" with the v1 empty passphrase
        // (asserted byte-exact in bip39.rs).
        let seed = hex64(
            "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a\
             5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4"
                .replace(' ', "")
                .as_str(),
        );
        let sign = derive_signing_seed(&seed);
        let ecdh = derive_ecdh_scalar(&seed);
        println!("SLIP-0010 project vectors (frozen by the §14 vector task):");
        println!("  seed (BIP-39, abandon×11+about, empty passphrase) = {}", to_hex(&seed[..]));
        println!("  m/111'/0'/0' (Ed25519 signing seed) = {}", to_hex(&sign[..]));
        println!("  m/111'/0'/1' (X25519 private scalar, raw) = {}", to_hex(&ecdh[..]));

        // Frozen values (this implementation is pinned byte-exact by the
        // published SLIP-0010 vectors above; the project paths are additionally
        // cross-checked against an independent Python HMAC-SHA512 derivation).
        assert_eq!(
            &sign[..],
            &hex32("718ffd7109b2318c3edea57191ed47c40b3bab75929efd82a319481e476ae2f5")[..]
        );
        assert_eq!(
            &ecdh[..],
            &hex32("cc680150d6f0c6be2aa1f4d43f86fa4a722c0474f27244bef8e98f06a1038126")[..]
        );
    }

    /// Hardened-only enforcement: an index carrying the hardened bit is a
    /// caller bug and rejected.
    #[test]
    fn slip0010_rejects_pre_hardened_indices() {
        let seed = [0u8; 64];
        assert!(matches!(
            derive_key_half(&seed, &[111, 0x8000_0000]),
            Err(Slip0010Error::IndexOutOfRange(0x8000_0000))
        ));
        assert!(derive_key_half(&seed, &[111, 0, 0]).is_ok());
    }
}
