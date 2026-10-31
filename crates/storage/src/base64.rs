//! 最小 base64 编解码（RFC 4648 标准字母表）。
//!
//! 依赖清单里没有 `base64` crate，而这里只需要对 32 字节 Ed25519 seed /
//! 公钥做编解码，因此保留一份无外部依赖的实现。解码同时接受 URL-safe
//! 字母表（`-`/`_`）与省略 padding 的写法，方便从环境变量粘贴密钥。

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 标准 base64 编码（带 `=` padding）。
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// 解码 base64；非法输入返回 `None`。
///
/// 允许 ASCII 空白（换行/空格）与可选的 padding，拒绝出现在中间或过多的 `=`。
pub(crate) fn decode(input: &str) -> Option<Vec<u8>> {
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    let mut out = Vec::with_capacity(cleaned.len() / 4 * 3);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    let mut data_len = 0usize;
    let mut padding = 0usize;
    for byte in cleaned {
        if byte == b'=' {
            padding += 1;
            continue;
        }
        if padding > 0 {
            // `=` 只能出现在末尾。
            return None;
        }
        let value = decode_byte(byte)?;
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        data_len += 1;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    let valid = if padding == 0 {
        // 不完整字节只允许剩 0/2/4 位（等价于末尾 0/2/3 个字符）。
        bits != 6
    } else {
        padding <= 2 && (data_len + padding).is_multiple_of(4) && bits == padding as u32 * 2
    };
    valid.then_some(out)
}

fn decode_byte(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn rfc4648_vectors() {
        let cases: [(&[u8], &str); 7] = [
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ];
        for (plain, encoded) in cases {
            assert_eq!(encode(plain), encoded);
            assert_eq!(decode(encoded).as_deref(), Some(plain));
        }
    }

    #[test]
    fn accepts_unpadded_and_url_safe_input() {
        assert_eq!(decode("Zg").as_deref(), Some(b"f".as_slice()));
        assert_eq!(decode("Zm9vYg").as_deref(), Some(b"foob".as_slice()));
        assert_eq!(decode("-_8=").as_deref(), Some(&[0xfb, 0xff][..]));
        assert_eq!(decode("Zg==\n").as_deref(), Some(b"f".as_slice()));
    }

    #[test]
    fn rejects_invalid_input() {
        for input in [
            "A", "Zg=", "Zg===", "Z===", "Zg==Zg==", "Zm9v!", "=", "Z g =",
        ] {
            assert!(decode(input).is_none(), "应拒绝 {input:?}");
        }
    }

    #[test]
    fn roundtrip_arbitrary_bytes() {
        let bytes: Vec<u8> = (0..=255u8).collect();
        assert_eq!(decode(&encode(&bytes)).as_deref(), Some(bytes.as_slice()));
    }
}
