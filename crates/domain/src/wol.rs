//! Wake-on-LAN：MAC 解析与 magic packet 构造（纯函数，便于单测）。
//!
//! 契约（`docs/wol.md`）：
//! - 控制端从机器最新快照 payload 的 `sections.agent.host.macs` 取目标 MAC；
//! - magic packet = 6 × `0xFF` + 目标 MAC 重复 16 次 = **102** 字节；
//! - 通过 UDP 广播（`255.255.255.255:9`）发往同一二层网络内的目标机。
//!
//! 这里只做纯计算，不碰网络/数据库；发送在 `qe-api` 的 admin handler 里完成。

use std::fmt;

use serde_json::Value;

/// magic packet 总长度：6 字节同步头 + 6 字节 MAC × 16。
pub const MAGIC_PACKET_LEN: usize = 102;
/// 同步头长度（6 字节 0xFF）。
pub const MAGIC_PACKET_SYNC_LEN: usize = 6;
/// MAC 重复次数。
pub const MAGIC_PACKET_MAC_REPEAT: usize = 16;

/// 规范化后的 MAC 地址（小写、冒号格式）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MacAddress(pub [u8; 6]);

impl MacAddress {
    /// 6 字节原始值。
    pub const fn bytes(self) -> [u8; 6] {
        self.0
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

/// 解析 MAC 文本：6 组 2 位十六进制，允许大写/前后空白；全零与非法值返回 `None`。
pub fn parse_mac(raw: &str) -> Option<MacAddress> {
    let trimmed = raw.trim();
    let parts: Vec<&str> = trimmed.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut bytes = [0u8; 6];
    for (index, part) in parts.iter().enumerate() {
        if part.len() != 2 {
            return None;
        }
        bytes[index] = u8::from_str_radix(part, 16).ok()?;
    }
    if bytes == [0u8; 6] {
        return None;
    }
    Some(MacAddress(bytes))
}

/// 从机器快照 payload 提取可用 MAC：`sections.agent.host.macs`。
///
/// 容忍脏数据：非数组/非字符串/非法 MAC 一律忽略；结果按 MAC 排序去重。
pub fn macs_from_payload(payload: &Value) -> Vec<MacAddress> {
    let list = payload
        .get("sections")
        .and_then(|sections| sections.get("agent"))
        .and_then(|agent| agent.get("host"))
        .and_then(|host| host.get("macs"))
        .and_then(Value::as_array);
    let mut macs: Vec<MacAddress> = list
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(parse_mac)
        .collect();
    macs.sort();
    macs.dedup();
    macs
}

/// 构造 Wake-on-LAN magic packet（102 字节）。
pub fn magic_packet(mac: MacAddress) -> [u8; MAGIC_PACKET_LEN] {
    let mut packet = [0u8; MAGIC_PACKET_LEN];
    packet[..MAGIC_PACKET_SYNC_LEN].fill(0xFF);
    for chunk in packet[MAGIC_PACKET_SYNC_LEN..].chunks_exact_mut(6) {
        chunk.copy_from_slice(&mac.bytes());
    }
    packet
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_lower_upper_and_whitespace() {
        let expected = MacAddress([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
        assert_eq!(parse_mac("aa:bb:cc:dd:ee:ff"), Some(expected));
        assert_eq!(parse_mac(" AA:BB:CC:DD:EE:FF\n"), Some(expected));
        assert_eq!(expected.to_string(), "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn rejects_zero_and_malformed() {
        for raw in [
            "00:00:00:00:00:00",
            "",
            "aa:bb:cc:dd:ee",
            "aa:bb:cc:dd:ee:ff:00",
            "zz:bb:cc:dd:ee:ff",
            "a:bb:cc:dd:ee:ff",
            "-1:bb:cc:dd:ee:ff",
        ] {
            assert_eq!(parse_mac(raw), None, "应拒绝：{raw}");
        }
    }

    #[test]
    fn magic_packet_layout_is_contract_exact() {
        let mac = MacAddress([0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e]);
        let packet = magic_packet(mac);
        assert_eq!(packet.len(), 102);
        assert_eq!(packet.len(), MAGIC_PACKET_LEN);
        assert_eq!(packet[..6], [0xFF; 6], "同步头必须是 6 个 0xFF");
        assert_eq!(packet[6..12], mac.bytes(), "同步头后紧跟目标 MAC");
        for repeat in 0..MAGIC_PACKET_MAC_REPEAT {
            let start = MAGIC_PACKET_SYNC_LEN + repeat * 6;
            assert_eq!(
                packet[start..start + 6],
                mac.bytes(),
                "第 {repeat} 次 MAC 重复不匹配"
            );
        }
        // 再确认没有把 0xFF 误填进 MAC 区（全 0xFF 的 MAC 本就被拒绝）。
        assert_eq!(packet.iter().filter(|byte| **byte == 0xFF).count(), 6);
    }

    #[test]
    fn macs_from_payload_sorts_dedups_and_ignores_garbage() {
        let payload = json!({
            "sections": {
                "agent": {
                    "host": {
                        "macs": [
                            "AA:BB:CC:DD:EE:02",
                            "aa:bb:cc:dd:ee:01",
                            "aa:bb:cc:dd:ee:02",
                            "00:00:00:00:00:00",
                            "not-a-mac",
                            42
                        ]
                    }
                }
            }
        });
        let macs: Vec<String> = macs_from_payload(&payload)
            .into_iter()
            .map(|mac| mac.to_string())
            .collect();
        assert_eq!(
            macs,
            vec![
                "aa:bb:cc:dd:ee:01".to_owned(),
                "aa:bb:cc:dd:ee:02".to_owned()
            ]
        );
    }

    #[test]
    fn macs_from_payload_missing_sections_returns_empty() {
        assert!(macs_from_payload(&json!({})).is_empty());
        assert!(macs_from_payload(&json!({"sections": {}})).is_empty());
        assert!(macs_from_payload(&json!({"sections": {"agent": {"host": {}}}})).is_empty());
        assert!(
            macs_from_payload(&json!({"sections": {"agent": {"host": {"macs": "nope"}}}}))
                .is_empty()
        );
    }
}
