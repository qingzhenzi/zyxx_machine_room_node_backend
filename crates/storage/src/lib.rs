//! S3 兼容存储适配（SigV4 预签名 + 对象读写 + manifest 签名）。
//!
//! P0 实现契约见 `docs/storage-design.md` §14.1：
//!
//! - [`StorageService::presign_get`] / [`StorageService::presign_put`] 生成 SigV4 query
//!   预签名 URL（短期下载/上传，凭证不出服务端）；
//! - [`StorageService::get_bytes`] / [`StorageService::put_bytes`] 供服务端内部读写对象
//!   （预签名 URL + reqwest，GET 响应体上限 64 MiB）；
//! - [`StorageService::sign_manifest`] / [`StorageService::verify_manifest`] 用 Ed25519
//!   对 manifest 原始字节签名/验签，节点侧用 [`StorageService::public_key_base64`] 校验。
//!
//! 设计要点：签名只覆盖 `host`，与 MinIO / RustFS / Ceph 等 S3 兼容实现一致；
//! `endpoint` 不允许携带路径前缀，`path_style=true` 时对象地址为
//! `{endpoint}/{bucket}/{key}`，否则为 `{bucket}.{host}/{key}`。

#![deny(unsafe_code)]

use sha2::{Digest, Sha256};

mod base64;
mod service;
mod sigv4;

pub use service::{PresignedUrl, StorageOptions, StorageService};

/// 计算 SHA-256 并以小写十六进制返回。
///
/// 对象内容寻址（`objects/{sha256}`、`manifests/{sha256}.json`）与 API 层
/// 登记对象/校验分片共用同一实现，避免各处各写一份。
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::sha256_hex;

    #[test]
    fn sha256_hex_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
