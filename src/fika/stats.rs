use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone)]
pub struct SessionStats {
    pub sent_packets: u64,
    pub sent_data_bytes: u64,
    pub received_packets: u64,
    pub received_data_bytes: u64,
    pub packet_loss_percent: f32,
    pub time_in_raid_seconds: u32,
}

static STATS_HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Fika Server Session Statistics").expect("valid regex"));
static SENT_PACKETS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Sent packets:\s*(\d+)").expect("valid regex"));
static SENT_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Sent data:\s*([\d.]+)\s*(KB|MB|GB)").expect("valid regex"));
static RECV_PACKETS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Received packets:\s*(\d+)").expect("valid regex"));
static RECV_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Received data:\s*([\d.]+)\s*(KB|MB|GB)").expect("valid regex"));
static PACKET_LOSS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Packet loss:\s*([\d.]+)%").expect("valid regex"));
static TIME_IN_RAID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Time in raid:\s*(\d+)h\s*(\d+)m\s*(\d+)s").expect("valid regex"));

fn parse_data_bytes(value: f64, unit: &str) -> u64 {
    match unit {
        "KB" => (value * 1024.0) as u64,
        "MB" => (value * 1024.0 * 1024.0) as u64,
        "GB" => (value * 1024.0 * 1024.0 * 1024.0) as u64,
        _ => value as u64,
    }
}

pub fn parse_session_stats(text: &str) -> Option<SessionStats> {
    if !STATS_HEADER.is_match(text) {
        return None;
    }

    let sent_packets = SENT_PACKETS
        .captures(text)?
        .get(1)?
        .as_str()
        .parse::<u64>()
        .ok()?;

    let sent_data_caps = SENT_DATA.captures(text)?;
    let sent_data_value = sent_data_caps.get(1)?.as_str().parse::<f64>().ok()?;
    let sent_data_unit = sent_data_caps.get(2)?.as_str();
    let sent_data_bytes = parse_data_bytes(sent_data_value, sent_data_unit);

    let received_packets = RECV_PACKETS
        .captures(text)?
        .get(1)?
        .as_str()
        .parse::<u64>()
        .ok()?;

    let recv_data_caps = RECV_DATA.captures(text)?;
    let recv_data_value = recv_data_caps.get(1)?.as_str().parse::<f64>().ok()?;
    let recv_data_unit = recv_data_caps.get(2)?.as_str();
    let received_data_bytes = parse_data_bytes(recv_data_value, recv_data_unit);

    let packet_loss_percent = PACKET_LOSS
        .captures(text)?
        .get(1)?
        .as_str()
        .parse::<f32>()
        .ok()?;

    let time_caps = TIME_IN_RAID.captures(text)?;
    let hours = time_caps.get(1)?.as_str().parse::<u32>().ok()?;
    let minutes = time_caps.get(2)?.as_str().parse::<u32>().ok()?;
    let seconds = time_caps.get(3)?.as_str().parse::<u32>().ok()?;
    let time_in_raid_seconds = hours * 3600 + minutes * 60 + seconds;

    Some(SessionStats {
        sent_packets,
        sent_data_bytes,
        received_packets,
        received_data_bytes,
        packet_loss_percent,
        time_in_raid_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_session_stats_from_log() {
        let log = r#"
[Info   :Fika.Server] ..:: Fika Server Session Statistics ::..
[Info   :Fika.Server] Sent packets: 80337
[Info   :Fika.Server] Sent data: 21.48 MB
[Info   :Fika.Server] Received packets: 76119
[Info   :Fika.Server] Received data: 4.05 MB
[Info   :Fika.Server] Packet loss: 0%
[Info   :Fika.Server] Time in raid: 00h 22m 32s
"#;
        let stats = parse_session_stats(log).unwrap();
        assert_eq!(stats.sent_packets, 80337);
        assert_eq!(stats.sent_data_bytes, (21.48 * 1024.0 * 1024.0) as u64);
        assert_eq!(stats.received_packets, 76119);
        assert_eq!(stats.received_data_bytes, (4.05 * 1024.0 * 1024.0) as u64);
        assert!(stats.packet_loss_percent < 0.01);
        assert_eq!(stats.time_in_raid_seconds, 22 * 60 + 32);
    }

    #[test]
    fn parse_returns_none_for_no_stats() {
        assert!(parse_session_stats("some random log output").is_none());
    }

    #[test]
    fn parse_handles_kb_units() {
        let log = r#"
[Info   :Fika.Server] ..:: Fika Server Session Statistics ::..
[Info   :Fika.Server] Sent packets: 100
[Info   :Fika.Server] Sent data: 512 KB
[Info   :Fika.Server] Received packets: 50
[Info   :Fika.Server] Received data: 256 KB
[Info   :Fika.Server] Packet loss: 1.5%
[Info   :Fika.Server] Time in raid: 01h 05m 15s
"#;
        let stats = parse_session_stats(log).unwrap();
        assert_eq!(stats.sent_data_bytes, 512 * 1024);
        assert_eq!(stats.received_data_bytes, 256 * 1024);
        assert!((stats.packet_loss_percent - 1.5).abs() < 0.01);
        assert_eq!(stats.time_in_raid_seconds, 3600 + 5 * 60 + 15);
    }

    #[test]
    fn parse_handles_gb_units() {
        let log = r#"
[Info   :Fika.Server] ..:: Fika Server Session Statistics ::..
[Info   :Fika.Server] Sent packets: 1000000
[Info   :Fika.Server] Sent data: 2.5 GB
[Info   :Fika.Server] Received packets: 500000
[Info   :Fika.Server] Received data: 1.2 GB
[Info   :Fika.Server] Packet loss: 0.05%
[Info   :Fika.Server] Time in raid: 02h 30m 00s
"#;
        let stats = parse_session_stats(log).unwrap();
        assert_eq!(
            stats.sent_data_bytes,
            (2.5 * 1024.0 * 1024.0 * 1024.0) as u64
        );
        assert_eq!(
            stats.received_data_bytes,
            (1.2 * 1024.0 * 1024.0 * 1024.0) as u64
        );
        assert_eq!(stats.time_in_raid_seconds, 2 * 3600 + 30 * 60);
    }

    #[test]
    fn parse_returns_none_for_partial_stats() {
        let log = r#"
[Info   :Fika.Server] ..:: Fika Server Session Statistics ::..
[Info   :Fika.Server] Sent packets: 80337
[Info   :Fika.Server] Sent data: 21.48 MB
"#;
        assert!(parse_session_stats(log).is_none());
    }
}
