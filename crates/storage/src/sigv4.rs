//! AWS Signature Version 4 的 query 预签名（S3）。
//!
//! 只实现 P0 需要的 `GET`/`PUT` 预签名：`UNSIGNED-PAYLOAD`、仅签 `host`，
//! 与 MinIO / RustFS / Ceph 的 S3 兼容实现一致。算法见
//! [AWS docs](https://docs.aws.amazon.com/AmazonS3/latest/API/sigv4-query-string-auth.html)。

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use sha2::Sha256;

/// RFC 3986 unreserved 之外全部编码（`-._~` 保留）。
const AWS_URI_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// 对象路径用：保留 `/` 作为分隔符。
const AWS_PATH_SET: &AsciiSet = &AWS_URI_SET.remove(b'/');

const ALGORITHM: &str = "AWS4-HMAC-SHA256";
const SERVICE: &str = "s3";
const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";
const SIGNED_HEADERS: &str = "host";

/// 对单个路径分量做 SigV4 编码（保留 `/`）。
pub(crate) fn encode_path(value: &str) -> String {
    utf8_percent_encode(value, AWS_PATH_SET).to_string()
}

/// 对 bucket 等单个分量做 SigV4 编码（`/` 也会被编码）。
pub(crate) fn encode_component(value: &str) -> String {
    utf8_percent_encode(value, AWS_URI_SET).to_string()
}

/// 一次 query 预签名所需的全部输入。
pub(crate) struct QueryPresign<'a> {
    pub scheme: &'a str,
    /// 参与签名的 `Host` 头（可能带端口）。
    pub host: &'a str,
    /// 已编码的 canonical URI，例如 `/bucket/objects/abc`。
    pub path: &'a str,
    pub method: &'a str,
    pub access_key: &'a str,
    pub secret_key: &'a str,
    pub region: &'a str,
    pub timestamp: DateTime<Utc>,
    pub expires_secs: u64,
}

/// 生成带 `X-Amz-Signature` 的预签名 URL。
pub(crate) fn presign_url(input: &QueryPresign<'_>) -> String {
    let date_stamp = input.timestamp.format("%Y%m%d").to_string();
    let amz_date = input.timestamp.format("%Y%m%dT%H%M%SZ").to_string();
    let scope = format!("{date_stamp}/{}/{SERVICE}/aws4_request", input.region);
    let credential = format!("{}/{}", input.access_key, scope);

    let mut params = [
        ("X-Amz-Algorithm", ALGORITHM.to_owned()),
        ("X-Amz-Credential", credential),
        ("X-Amz-Date", amz_date.clone()),
        ("X-Amz-Expires", input.expires_secs.to_string()),
        ("X-Amz-SignedHeaders", SIGNED_HEADERS.to_owned()),
    ];
    params.sort_unstable_by(|left, right| left.0.cmp(right.0));

    let canonical_query = params
        .iter()
        .map(|(name, value)| format!("{}={}", encode_component(name), encode_component(value)))
        .collect::<Vec<_>>()
        .join("&");

    let canonical_request = format!(
        "{method}\n{path}\n{query}\nhost:{host}\n\n{signed}\n{payload}",
        method = input.method,
        path = input.path,
        query = canonical_query,
        host = input.host,
        signed = SIGNED_HEADERS,
        payload = UNSIGNED_PAYLOAD,
    );

    let string_to_sign = format!(
        "{ALGORITHM}\n{amz_date}\n{scope}\n{request_hash}",
        request_hash = crate::sha256_hex(canonical_request.as_bytes()),
    );
    let signature = hex::encode(hmac_sha256(
        &signing_key(input.secret_key, &date_stamp, input.region),
        string_to_sign.as_bytes(),
    ));

    format!(
        "{scheme}://{host}{path}?{canonical_query}&X-Amz-Signature={signature}",
        scheme = input.scheme,
        host = input.host,
        path = input.path,
    )
}

fn signing_key(secret_key: &str, date_stamp: &str, region: &str) -> [u8; 32] {
    let initial = format!("AWS4{secret_key}");
    let date_key = hmac_sha256(initial.as_bytes(), date_stamp.as_bytes());
    let region_key = hmac_sha256(&date_key, region.as_bytes());
    let service_key = hmac_sha256(&region_key, SERVICE.as_bytes());
    hmac_sha256(&service_key, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC 接受任意长度密钥");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

#[cfg(test)]
mod tests {
    use super::{QueryPresign, encode_component, encode_path, presign_url};
    use chrono::{TimeZone, Utc};

    /// AWS 官方 SigV4 query-presign 示例。
    #[test]
    fn aws_reference_presigned_url() {
        let timestamp = Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
        let url = presign_url(&QueryPresign {
            scheme: "https",
            host: "examplebucket.s3.amazonaws.com",
            path: "/test.txt",
            method: "GET",
            access_key: "AKIAIOSFODNN7EXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
            timestamp,
            expires_secs: 86_400,
        });
        assert_eq!(
            url,
            "https://examplebucket.s3.amazonaws.com/test.txt?\
             X-Amz-Algorithm=AWS4-HMAC-SHA256&\
             X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&\
             X-Amz-Date=20130524T000000Z&\
             X-Amz-Expires=86400&\
             X-Amz-SignedHeaders=host&\
             X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        );
    }

    #[test]
    fn encodes_path_and_components() {
        assert_eq!(encode_path("objects/a b+c.txt"), "objects/a%20b%2Bc.txt");
        assert_eq!(encode_path("objects/中文"), "objects/%E4%B8%AD%E6%96%87");
        assert_eq!(encode_component("a/b"), "a%2Fb");
        assert_eq!(encode_component("a-b_c.d~e"), "a-b_c.d~e");
    }
}
