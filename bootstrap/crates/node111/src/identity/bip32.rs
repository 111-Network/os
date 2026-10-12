//! BIP-32 private derivation over secp256k1 (CKDpriv), minimal in-crate
//! implementation on hmac+sha2+k256 — validated against the published NIP-06
//! vectors (see identity/mod.rs tests).
//!
//! Only what NIP-06 needs: master from BIP-39 seed ("Bitcoin seed" HMAC key),
//! hardened + normal CKDpriv, paths like `m/44'/1237'/0'/0/0`.

use hmac::{Hmac, Mac};
use k256::elliptic_curve::ff::PrimeField;
use k256::elliptic_curve::Scalar;

type HmacSha512 = Hmac<sha2::Sha512>;
type SecpScalar = Scalar<k256::Secp256k1>;

const HARDENED: u32 = 0x8000_0000;

fn hmac_sha512(key: &[u8], data: &[u8]) -> [u8; 64] {
    let mut mac = <HmacSha512 as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn scalar_from_bytes(b: &[u8; 32]) -> anyhow::Result<SecpScalar> {
    Option::from(SecpScalar::from_repr((*b).into()))
        .ok_or_else(|| anyhow::anyhow!("BIP-32: IL not a valid secp256k1 scalar"))
}

struct Node {
    privkey: [u8; 32],
    chain: [u8; 32],
}

impl Node {
    /// BIP-32 accepts arbitrary-length seeds; NIP-06's are always 64 bytes
    /// (the public API stays `&[u8; 64]`). Tests drive master directly.
    fn master(seed: &[u8]) -> anyhow::Result<Node> {
        let i = hmac_sha512(b"Bitcoin seed", seed);
        let (il, ir) = i.split_at(32);
        let privkey: [u8; 32] = il.try_into().unwrap();
        // Master key must be a valid scalar (IL = 0 or >= n → invalid per BIP-32).
        scalar_from_bytes(&privkey)?;
        Ok(Node {
            privkey,
            chain: ir.try_into().unwrap(),
        })
    }

    fn ckd_priv(&self, index: u32) -> anyhow::Result<Node> {
        let mut data = Vec::with_capacity(37);
        if index & HARDENED != 0 {
            data.push(0x00);
            data.extend_from_slice(&self.privkey);
        } else {
            // Normal derivation commits to the compressed parent public key.
            let secret = k256::SecretKey::from_slice(&self.privkey)
                .map_err(|_| anyhow::anyhow!("BIP-32: invalid parent key"))?;
            data.extend_from_slice(&secret.public_key().to_sec1_bytes());
        }
        data.extend_from_slice(&index.to_be_bytes());

        let i = hmac_sha512(&self.chain, &data);
        let (il, ir) = i.split_at(32);
        let il_scalar = scalar_from_bytes(il.try_into().unwrap())?;

        let parent = k256::SecretKey::from_slice(&self.privkey)
            .map_err(|_| anyhow::anyhow!("BIP-32: invalid parent key"))?;
        let parent_scalar = parent.to_nonzero_scalar();
        // child = (IL + parent) mod n; zero child is cryptographically
        // impossible-in-practice and refused per BIP-32 (proceed to next index).
        let child_scalar = *parent_scalar.as_ref() + il_scalar;
        if bool::from(child_scalar.is_zero()) {
            anyhow::bail!("BIP-32: derived zero child key (invalid per BIP-32)");
        }
        Ok(Node {
            privkey: child_scalar.to_bytes().into(),
            chain: ir.try_into().unwrap(),
        })
    }
}

/// Derive the private key at a NIP-06-style path (`m/44'/1237'/0'/0/0`) from a
/// BIP-39 seed (64 bytes). `'`/`h`/`H` suffix = hardened.
pub fn derive_private_key(seed: &[u8; 64], path: &str) -> anyhow::Result<[u8; 32]> {
    let mut segments = path.split('/');
    anyhow::ensure!(
        segments.next() == Some("m"),
        "path must start with 'm': {path:?}"
    );
    let mut node = Node::master(seed)?;
    for seg in segments {
        let (num, hardened) = match seg.strip_suffix(['\'', 'h', 'H']) {
            Some(n) => (n, true),
            None => (seg, false),
        };
        let idx: u32 = num
            .parse()
            .map_err(|_| anyhow::anyhow!("bad path segment {seg:?}"))?;
        anyhow::ensure!(idx < HARDENED, "path segment {seg:?} >= 2^31");
        node = node.ckd_priv(if hardened { idx | HARDENED } else { idx })?;
    }
    Ok(node.privkey)
}

#[cfg(test)]
mod tests {
    use super::*;

    // BIP-32 test vector 1 (from the BIP-32 spec): seed 000102...0e0f (16 bytes),
    // asserted private keys along m/0' and m/0'/1.
    #[test]
    fn bip32_vector1_m0h() {
        let seed = hexbytes("000102030405060708090a0b0c0d0e0f");
        let m = Node::master(&seed).unwrap();
        let m0h = m.ckd_priv(HARDENED).unwrap();
        assert_eq!(
            hex(&m0h.privkey),
            "edb2e14f9ee77d26dd93b4ecede8d16ed408ce149b6cd80b0715a2d911a0afea"
        );
        let m0h1 = m0h.ckd_priv(1).unwrap();
        assert_eq!(
            hex(&m0h1.privkey),
            "3c6cb8d0f6a264c91ea8b5030fadaa8e538b020f0a387421a12de9319dc93368"
        );
    }

    #[test]
    fn path_validation() {
        let seed = [0u8; 64];
        assert!(derive_private_key(&seed, "m").is_ok()); // master
        assert!(derive_private_key(&seed, "0/0").is_err()); // missing m
        assert!(derive_private_key(&seed, "m/x").is_err());
        assert!(derive_private_key(&seed, "m/2147483648").is_err());
        assert!(derive_private_key(&seed, "m/1h").is_ok()); // h suffix accepted
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
    fn hexbytes(s: &str) -> Vec<u8> {
        s.as_bytes()
            .chunks(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect()
    }
}
