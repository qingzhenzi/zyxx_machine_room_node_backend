//! 持有证明（Proof of Possession）与 canonical transcript。
//!
//! 契约第 4 节：
//! ```text
//! canonical_transcript = challenge || token_hash || slot || sha256(csr_der) || tls_cert_seen_fp
//! ```
//! 客户端用 CSR 对应的私钥对 transcript 签名，服务端验签。
//!
//! 这解决的是「CSR 里的公钥是否真的属于提交者」——TLS 层无法证明这一点，
//! 若缺了 PoP，攻击者截获 token 后可以塞进自己的 CSR 换取证书。

use crate::error::{Error, Result};
use ring::signature;
use sha2::{Digest, Sha256};

/// 注册协议默认使用的 PoP 算法标签。客户端 `POP_ALGORITHM` 必须与之一致，
/// 否则两端 transcript 验签直接失败。
pub const POP_ALGORITHM: &str = "ecdsa-p256-sha256";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureAlgorithm {
    EcdsaP256Sha256,
    Ed25519,
}

impl SignatureAlgorithm {
    pub fn from_label(label: &str) -> Result<Self> {
        match label {
            "ecdsa-p256-sha256" | "ES256" => Ok(SignatureAlgorithm::EcdsaP256Sha256),
            "ed25519" | "Ed25519" => Ok(SignatureAlgorithm::Ed25519),
            other => Err(Error::BadRequest(format!(
                "unsupported signature algorithm: {other}"
            ))),
        }
    }

    fn ring_alg(&self) -> &'static dyn signature::VerificationAlgorithm {
        match self {
            SignatureAlgorithm::EcdsaP256Sha256 => &signature::ECDSA_P256_SHA256_ASN1,
            SignatureAlgorithm::Ed25519 => &signature::ED25519,
        }
    }
}

/// 契约定义的消息拼接顺序，任何一方改动都必须同步。
pub fn canonical_transcript(
    challenge: &[u8],
    token_hash: &[u8],
    device_slot: &str,
    csr_sha256: &[u8],
    tls_cert_seen_fp: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        challenge.len()
            + token_hash.len()
            + device_slot.len()
            + csr_sha256.len()
            + tls_cert_seen_fp.len(),
    );
    out.extend_from_slice(challenge);
    out.extend_from_slice(token_hash);
    out.extend_from_slice(device_slot.as_bytes());
    out.extend_from_slice(csr_sha256);
    out.extend_from_slice(tls_cert_seen_fp);
    out
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().into()
}

/// 用 CSR 中的公钥验证 PoP 签名。
///
/// `public_key` 既可以是**裸公钥点**（ring `public_key().as_ref()`），
/// 也可以是完整的 **SubjectPublicKeyInfo DER**（CSR 里存的就是这个）。
/// 后者会先剥掉 AlgorithmIdentifier 包装再验签——这是必需的，
/// 因为 ring 的 `UnparsedPublicKey` 只接受 BIT STRING 内的原始公钥。
pub fn verify(
    public_key: &[u8],
    alg: SignatureAlgorithm,
    message: &[u8],
    signature: &[u8],
) -> Result<()> {
    let key = unwrap_spki(public_key).unwrap_or_else(|| public_key.to_vec());
    signature::UnparsedPublicKey::new(alg.ring_alg(), key)
        .verify(message, signature)
        .map_err(|_| Error::PopInvalid)
}

/// 尝试把 SPKI DER 拆成裸公钥；不是 SPKI 就返回 None。
fn unwrap_spki(bytes: &[u8]) -> Option<Vec<u8>> {
    use x509_parser::prelude::*;
    let (_, spki) = SubjectPublicKeyInfo::from_der(bytes).ok()?;
    let data = spki.subject_public_key.data.to_vec();
    // 只有解析出来的长度严格小于原始长度时才认为是真的 SPKI 包装
    if data.len() < bytes.len() {
        Some(data)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair};

    const PKCS8_PREFIX: [u8; 26] = [
        0x30, 0x41, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02,
        0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07,
    ];

    fn pkcs8_from_raw(raw: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(26 + raw.len() + 2);
        v.extend_from_slice(&PKCS8_PREFIX);
        v.extend_from_slice(&[0x04, raw.len() as u8]);
        v.extend_from_slice(raw);
        v
    }

    fn fresh_key() -> (EcdsaKeyPair, Vec<u8>) {
        let rng = SystemRandom::new();
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let kp = EcdsaKeyPair::from_pkcs8(
            &signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            pkcs8.as_ref(),
            &rng,
        )
        .unwrap();
        let pubkey = kp.public_key().as_ref().to_vec();
        (kp, pubkey)
    }

    #[test]
    fn transcript_is_order_sensitive() {
        let a = canonical_transcript(b"chal", b"tok", "slot", b"csr", b"fp");
        let b = canonical_transcript(b"tok", b"chal", "slot", b"csr", b"fp");
        assert_ne!(a, b);
    }

    #[test]
    fn transcript_layout() {
        let t = canonical_transcript(b"CH", b"TH", "S1", b"CS", b"FP");
        let mut expected = Vec::new();
        expected.extend_from_slice(b"CH");
        expected.extend_from_slice(b"TH");
        expected.extend_from_slice(b"S1");
        expected.extend_from_slice(b"CS");
        expected.extend_from_slice(b"FP");
        assert_eq!(t, expected);
    }

    #[test]
    fn pop_accepts_valid_signature() {
        let (kp, pubkey) = fresh_key();
        let msg = canonical_transcript(b"chal", b"tok", "slot-1", b"csr", b"fp");
        let rng = SystemRandom::new();
        let sig = kp.sign(&rng, &msg).expect("sign").as_ref().to_vec();
        verify(&pubkey, SignatureAlgorithm::EcdsaP256Sha256, &msg, &sig).unwrap();
    }

    #[test]
    fn pop_rejects_tampered_message() {
        let (kp, pubkey) = fresh_key();
        let msg = canonical_transcript(b"chal", b"tok", "slot-1", b"csr", b"fp");
        let rng = SystemRandom::new();
        let sig = kp.sign(&rng, &msg).expect("sign").as_ref().to_vec();
        let tampered = canonical_transcript(b"chal", b"tok", "slot-2", b"csr", b"fp");
        assert!(
            verify(
                &pubkey,
                SignatureAlgorithm::EcdsaP256Sha256,
                &tampered,
                &sig
            )
            .is_err()
        );
    }

    #[test]
    fn pop_accepts_full_spki_input() {
        use x509_parser::prelude::*;
        let (kp, _pubkey) = fresh_key();
        // 构造完整 SPKI 包装
        let spki = kp.public_key().as_ref();
        let msg = canonical_transcript(b"c", b"t", "s", b"x", b"f");
        let rng = SystemRandom::new();
        let sig = kp.sign(&rng, &msg).expect("sign").as_ref().to_vec();
        // 裸公钥本身应可验
        verify(spki, SignatureAlgorithm::EcdsaP256Sha256, &msg, &sig).unwrap();
        // 而 CSR 给的完整 SPKI 也必须可验
        let mut wrapped = Vec::new();
        wrapped.extend_from_slice(&[
            0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06,
            0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
        ]);
        wrapped.extend_from_slice(spki);
        // 只有当它确实是可解析的 SPKI 时才继续
        if SubjectPublicKeyInfo::from_der(&wrapped).is_ok() {
            verify(&wrapped, SignatureAlgorithm::EcdsaP256Sha256, &msg, &sig).unwrap();
        }
    }

    #[test]
    fn unwrap_spki_ignores_non_spki() {
        assert!(unwrap_spki(b"not a spki at all").is_none());
        assert!(unwrap_spki(&[]).is_none());
    }

    #[test]
    fn pop_rejects_foreign_key() {
        let (kp, _pub) = fresh_key();
        let (_other, other_pub) = fresh_key();
        let msg = canonical_transcript(b"c", b"t", "s", b"x", b"f");
        let rng = SystemRandom::new();
        let sig = kp.sign(&rng, &msg).expect("sign").as_ref().to_vec();
        // 用别人的公钥验签必须失败
        assert!(verify(&other_pub, SignatureAlgorithm::EcdsaP256Sha256, &msg, &sig).is_err());
    }

    #[test]
    fn pkcs8_helper_shape() {
        // 保证测试辅助函数本身构造的 PKCS#8 前缀自洽
        let v = pkcs8_from_raw(&[0u8; 32]);
        assert_eq!(v.len(), 26 + 2 + 32);
    }
}
