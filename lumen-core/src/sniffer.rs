//! Passive packet capture for monitoring LLM API traffic.
//!
//! Uses libpcap/BPF to observe TCP traffic on port 443 and identifies
//! LLM API flows via SNI (Server Name Indication) in TLS ClientHello.
//! No proxy, no TLS interception, no DNS resolution needed.
//! Tracks bytes per connection to estimate token usage.

use crate::aggregator::Aggregator;
use crate::parser::{self, LLMProvider};
use crate::traffic::{TrafficEntry, TrafficLog};
use anyhow::{Context, Result};
use chrono::Utc;
use etherparse::{NetSlice, SlicedPacket, TransportSlice};
use pcap::{Capture, Device};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

const MONITORED_EXACT: &[&str] = &[
    "api.openai.com",
    "api.anthropic.com",
    "generativelanguage.googleapis.com",
];

const MONITORED_SUFFIXES: &[&str] = &[".cursor.sh", ".cursorapi.com"];

const FLOW_IDLE_TIMEOUT_SECS: u64 = 10;
const FLUSH_INTERVAL_SECS: u64 = 5;

/// Captures the pcap data link type so we can parse frames correctly.
#[derive(Debug)]
enum LinkType {
    Ethernet,
    RawIp,
    BsdNull,
    Unknown(()),
}

impl LinkType {
    fn from_pcap(dl: pcap::Linktype) -> Self {
        match dl.0 {
            1 => LinkType::Ethernet,
            0 => LinkType::BsdNull,
            12 | 101 => LinkType::RawIp,
            _other => LinkType::Unknown(()),
        }
    }

    fn parse<'a>(&self, data: &'a [u8]) -> Option<SlicedPacket<'a>> {
        match self {
            LinkType::Ethernet => SlicedPacket::from_ethernet(data).ok(),
            LinkType::RawIp => SlicedPacket::from_ip(data).ok(),
            LinkType::BsdNull => {
                if data.len() < 4 {
                    return None;
                }
                let family = u32::from_ne_bytes([data[0], data[1], data[2], data[3]]);
                if family == 2 || family == 30 {
                    SlicedPacket::from_ip(&data[4..]).ok()
                } else {
                    None
                }
            }
            LinkType::Unknown(_) => SlicedPacket::from_ip(data)
                .ok()
                .or_else(|| SlicedPacket::from_ethernet(data).ok()),
        }
    }
}

/// Canonical flow identifier: always normalized to (local_port, remote_ip, remote_port).
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
struct FlowKey {
    local_port: u16,
    remote_ip: IpAddr,
    remote_port: u16,
}

/// Tracks the state of a TCP flow through its lifecycle.
enum FlowStatus {
    /// Waiting for SNI from TLS ClientHello
    Pending,
    /// SNI matched a monitored hostname
    Monitored(String),
    /// SNI didn't match or couldn't be extracted — ignore this flow
    Ignored,
}

struct FlowState {
    status: FlowStatus,
    bytes_out: u64,
    bytes_in: u64,
    started: Instant,
    last_seen: Instant,
    first_payload_seen: bool,
}

pub struct PassiveSniffer {
    aggregator: Arc<Aggregator>,
    traffic_log: Arc<TrafficLog>,
    interface: Option<String>,
    local_ips: HashSet<IpAddr>,
    monitored_exact: HashSet<String>,
    monitored_suffixes: Vec<String>,
}

impl PassiveSniffer {
    pub fn new(
        aggregator: Arc<Aggregator>,
        traffic_log: Arc<TrafficLog>,
        interface: Option<String>,
    ) -> Self {
        let monitored_exact = MONITORED_EXACT.iter().map(|h| h.to_string()).collect();
        let monitored_suffixes = MONITORED_SUFFIXES.iter().map(|s| s.to_string()).collect();

        Self {
            aggregator,
            traffic_log,
            interface,
            local_ips: detect_local_ips(),
            monitored_exact,
            monitored_suffixes,
        }
    }

    fn is_monitored_hostname(&self, hostname: &str) -> bool {
        self.monitored_exact.contains(hostname)
            || self
                .monitored_suffixes
                .iter()
                .any(|s| hostname.ends_with(s.as_str()))
    }

    pub async fn start(self: Arc<Self>) -> Result<()> {
        let devices = match &self.interface {
            Some(name) => {
                let all = Device::list().context("Failed to list pcap devices")?;
                let dev = all
                    .into_iter()
                    .find(|d| d.name == *name)
                    .ok_or_else(|| anyhow::anyhow!("Interface {} not found", name))?;
                vec![dev]
            }
            None => find_capturable_interfaces()?,
        };

        if devices.is_empty() {
            anyhow::bail!("No capturable interfaces found");
        }

        info!("Local IPs: {:?}", self.local_ips.iter().collect::<Vec<_>>());
        info!(
            "Passive sniffer capturing on {} interface(s): {}",
            devices.len(),
            devices
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );

        // Shared state across all interface capture threads
        let flows = Arc::new(parking_lot::Mutex::new(HashMap::<FlowKey, FlowState>::new()));
        let ignored = Arc::new(parking_lot::Mutex::new(HashSet::<FlowKey>::new()));

        // Spawn a capture thread per interface
        for device in devices {
            let sniffer = self.clone();
            let flows = flows.clone();
            let ignored = ignored.clone();
            let dev_name = device.name.clone();

            std::thread::spawn(move || {
                if let Err(e) = sniffer.capture_loop(device, flows, ignored) {
                    warn!("Capture on {} failed: {}", dev_name, e);
                }
            });
        }

        // Stats + flush loop on the async side
        let mut last_flush = Instant::now();
        let mut last_stats = Instant::now();

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            if last_stats.elapsed().as_secs() >= 15 {
                let fl = flows.lock();
                let monitored_count = fl
                    .values()
                    .filter(|f| matches!(f.status, FlowStatus::Monitored(_)))
                    .count();
                let pending_count = fl
                    .values()
                    .filter(|f| matches!(f.status, FlowStatus::Pending))
                    .count();
                let ignored_count = ignored.lock().len();
                info!(
                    "Sniffer: {} monitored flows, {} pending, {} ignored",
                    monitored_count, pending_count, ignored_count,
                );
                last_stats = Instant::now();
            }

            if last_flush.elapsed().as_secs() >= FLUSH_INTERVAL_SECS {
                let mut fl = flows.lock();
                let mut ig = ignored.lock();
                self.flush_idle_flows(&mut fl, &mut ig);
                last_flush = Instant::now();
            }
        }
    }

    /// Blocking capture loop for a single interface. Runs on a dedicated thread.
    fn capture_loop(
        &self,
        device: Device,
        flows: Arc<parking_lot::Mutex<HashMap<FlowKey, FlowState>>>,
        ignored: Arc<parking_lot::Mutex<HashSet<FlowKey>>>,
    ) -> Result<()> {
        let dev_name = device.name.clone();

        let mut cap = Capture::from_device(device)
            .context("Failed to open capture device")?
            .promisc(false)
            .snaplen(512)
            .timeout(100)
            .open()
            .with_context(|| format!("Failed to activate capture on {}", dev_name))?;

        cap.filter("tcp port 443", true)
            .with_context(|| format!("Failed to set BPF filter on {}", dev_name))?;

        let datalink = cap.get_datalink();
        let link_type = LinkType::from_pcap(datalink);
        info!(
            "Capture active on {} — datalink: {:?} ({})",
            dev_name, link_type, datalink.0
        );

        loop {
            match cap.next_packet() {
                Ok(packet) => {
                    let mut fl = flows.lock();
                    let mut ig = ignored.lock();
                    self.process_packet(packet.data, &link_type, &mut fl, &mut ig);
                }
                Err(pcap::Error::TimeoutExpired) => {}
                Err(e) => {
                    warn!("Capture error on {}: {}", dev_name, e);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
    }

    fn process_packet(
        &self,
        data: &[u8],
        link_type: &LinkType,
        flows: &mut HashMap<FlowKey, FlowState>,
        ignored_flows: &mut HashSet<FlowKey>,
    ) {
        let parsed = match link_type.parse(data) {
            Some(p) => p,
            None => return,
        };

        let (src_ip, dst_ip, ip_payload_len) = match &parsed.net {
            Some(NetSlice::Ipv4(ipv4)) => {
                let h = ipv4.header();
                let total = h.total_len() as usize;
                let hdr_len = h.ihl() as usize * 4;
                (
                    IpAddr::V4(h.source_addr()),
                    IpAddr::V4(h.destination_addr()),
                    total.saturating_sub(hdr_len),
                )
            }
            Some(NetSlice::Ipv6(ipv6)) => {
                let h = ipv6.header();
                (
                    IpAddr::V6(h.source_addr()),
                    IpAddr::V6(h.destination_addr()),
                    h.payload_length() as usize,
                )
            }
            None => return,
        };

        let (src_port, dst_port, tcp_hdr_len, tcp_payload) = match &parsed.transport {
            Some(TransportSlice::Tcp(tcp)) => {
                let hdr_len = tcp.data_offset() as usize * 4;
                (
                    tcp.source_port(),
                    tcp.destination_port(),
                    hdr_len,
                    tcp.payload(),
                )
            }
            _ => return,
        };

        // Skip loopback (dedup with proxy)
        if dst_ip.is_loopback() || src_ip.is_loopback() {
            return;
        }

        let actual_payload_len = ip_payload_len.saturating_sub(tcp_hdr_len);

        // Determine direction
        let is_outbound = self.local_ips.contains(&src_ip);
        let is_inbound = self.local_ips.contains(&dst_ip);

        if !is_outbound && !is_inbound {
            return;
        }

        let (flow_key, bytes_out, bytes_in) = if is_outbound {
            (
                FlowKey {
                    local_port: src_port,
                    remote_ip: dst_ip,
                    remote_port: dst_port,
                },
                actual_payload_len as u64,
                0u64,
            )
        } else {
            (
                FlowKey {
                    local_port: dst_port,
                    remote_ip: src_ip,
                    remote_port: src_port,
                },
                0u64,
                actual_payload_len as u64,
            )
        };

        // Fast path: skip flows we've already determined are uninteresting
        if ignored_flows.contains(&flow_key) {
            return;
        }

        let now = Instant::now();
        let flow = flows.entry(flow_key.clone()).or_insert_with(|| FlowState {
            status: FlowStatus::Pending,
            bytes_out: 0,
            bytes_in: 0,
            started: now,
            last_seen: now,
            first_payload_seen: false,
        });

        flow.last_seen = now;

        // Try SNI extraction on first outbound packet with payload
        if !flow.first_payload_seen && is_outbound && actual_payload_len > 0 {
            flow.first_payload_seen = true;

            if let Some(sni) = extract_sni_from_packet(tcp_payload) {
                if self.is_monitored_hostname(&sni) {
                    info!("SNI match: {} ({})", sni, flow_key.remote_ip);
                    flow.status = FlowStatus::Monitored(sni);
                } else {
                    debug!("SNI skip: {}", sni);
                    flow.status = FlowStatus::Ignored;
                    let removed = flows.remove(&flow_key);
                    drop(removed);
                    ignored_flows.insert(flow_key);
                    return;
                }
            }
            // No SNI found (maybe not TLS, or truncated) — keep as Pending for now
        }

        // Only accumulate bytes for monitored or pending flows
        match &flow.status {
            FlowStatus::Monitored(_) | FlowStatus::Pending => {
                flow.bytes_out += bytes_out;
                flow.bytes_in += bytes_in;
            }
            FlowStatus::Ignored => {}
        }
    }

    fn flush_idle_flows(
        &self,
        flows: &mut HashMap<FlowKey, FlowState>,
        ignored_flows: &mut HashSet<FlowKey>,
    ) {
        let now = Instant::now();
        let idle_keys: Vec<FlowKey> = flows
            .iter()
            .filter(|(_, f)| now.duration_since(f.last_seen).as_secs() >= FLOW_IDLE_TIMEOUT_SECS)
            .map(|(k, _)| k.clone())
            .collect();

        for key in idle_keys {
            if let Some(flow) = flows.remove(&key) {
                match flow.status {
                    FlowStatus::Monitored(hostname) => {
                        if flow.bytes_out == 0 && flow.bytes_in == 0 {
                            continue;
                        }

                        let provider = detect_provider_from_host(&hostname);
                        let latency_ms =
                            flow.last_seen.duration_since(flow.started).as_millis() as u64;

                        let mut data_captured = vec!["passive".to_string()];

                        if let Some(provider) = provider {
                            if flow.bytes_out > 50 || flow.bytes_in > 200 {
                                let usage = parser::estimate_usage_from_bytes(
                                    flow.bytes_out,
                                    flow.bytes_in,
                                );
                                info!(
                                    "Passive: {} ~{}in/~{}out tokens ({}B↑ {}B↓, {:.1}s)",
                                    hostname,
                                    usage.input_tokens,
                                    usage.output_tokens,
                                    flow.bytes_out,
                                    flow.bytes_in,
                                    latency_ms as f64 / 1000.0,
                                );
                                data_captured.push("tokens_in".to_string());
                                data_captured.push("tokens_out".to_string());
                                data_captured.push("estimated".to_string());
                                self.aggregator
                                    .record_usage(provider, "unknown", &hostname, usage);
                            }
                        }

                        self.traffic_log.record(TrafficEntry {
                            id: uuid::Uuid::new_v4().to_string(),
                            timestamp: Utc::now(),
                            host: hostname,
                            method: "PASSIVE".to_string(),
                            path: format!(
                                ":{} ({}B out, {}B in)",
                                key.remote_port, flow.bytes_out, flow.bytes_in
                            ),
                            status: 200,
                            request_bytes: flow.bytes_out,
                            response_bytes: flow.bytes_in,
                            is_monitored: true,
                            data_captured,
                            latency_ms,
                        });
                    }
                    FlowStatus::Pending => {
                        // Timed out without SNI — not interesting
                    }
                    FlowStatus::Ignored => {}
                }
            }
        }

        // Prune ignored flows older than 60s to bound memory
        if ignored_flows.len() > 10_000 {
            ignored_flows.clear();
        }
    }
}

fn detect_provider_from_host(hostname: &str) -> Option<LLMProvider> {
    if hostname.contains("openai") {
        Some(LLMProvider::OpenAI)
    } else if hostname.contains("anthropic") {
        Some(LLMProvider::Anthropic)
    } else if hostname.contains("googleapis") {
        Some(LLMProvider::Google)
    } else if hostname.contains("cursor") {
        Some(LLMProvider::Cursor)
    } else {
        None
    }
}

/// Extract SNI hostname from raw TLS ClientHello bytes (TCP payload).
fn extract_sni_from_packet(payload: &[u8]) -> Option<String> {
    if payload.len() < 5 || payload[0] != 0x16 {
        return None;
    }

    let record_len = ((payload[3] as usize) << 8) | payload[4] as usize;
    let handshake_end = std::cmp::min(5 + record_len, payload.len());
    let handshake = &payload[5..handshake_end];

    if handshake.is_empty() || handshake[0] != 0x01 {
        return None;
    }
    if handshake.len() < 38 {
        return None;
    }

    let mut pos = 38;

    if pos >= handshake.len() {
        return None;
    }
    let session_id_len = handshake[pos] as usize;
    pos += 1 + session_id_len;

    if pos + 2 > handshake.len() {
        return None;
    }
    let cipher_len = ((handshake[pos] as usize) << 8) | handshake[pos + 1] as usize;
    pos += 2 + cipher_len;

    if pos >= handshake.len() {
        return None;
    }
    let comp_len = handshake[pos] as usize;
    pos += 1 + comp_len;

    if pos + 2 > handshake.len() {
        return None;
    }
    let ext_total = ((handshake[pos] as usize) << 8) | handshake[pos + 1] as usize;
    pos += 2;
    let ext_end = std::cmp::min(pos + ext_total, handshake.len());

    while pos + 4 <= ext_end {
        let ext_type = ((handshake[pos] as u16) << 8) | handshake[pos + 1] as u16;
        let ext_len = ((handshake[pos + 2] as usize) << 8) | handshake[pos + 3] as usize;
        pos += 4;

        if ext_type == 0x0000 {
            if pos + 2 > ext_end {
                break;
            }
            pos += 2;
            if pos + 3 > ext_end {
                break;
            }
            let name_type = handshake[pos];
            let name_len = ((handshake[pos + 1] as usize) << 8) | handshake[pos + 2] as usize;
            pos += 3;
            if name_type == 0 && pos + name_len <= ext_end {
                return std::str::from_utf8(&handshake[pos..pos + name_len])
                    .ok()
                    .map(String::from);
            }
            break;
        }
        pos += ext_len;
    }

    None
}

/// Find all non-loopback interfaces that have at least one IPv4 address.
fn find_capturable_interfaces() -> Result<Vec<Device>> {
    let devices = Device::list().context("Failed to list pcap devices")?;
    let mut result = Vec::new();

    for dev in devices {
        if dev.name == "lo0" || dev.name.starts_with("lo") {
            continue;
        }
        let has_ipv4 = dev
            .addresses
            .iter()
            .any(|a| matches!(a.addr, IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_unspecified()));
        if has_ipv4 {
            result.push(dev);
        }
    }

    Ok(result)
}

fn detect_local_ips() -> HashSet<IpAddr> {
    let mut ips = HashSet::new();
    ips.insert(IpAddr::V4(Ipv4Addr::LOCALHOST));

    if let Ok(devices) = Device::list() {
        for device in devices {
            for addr in &device.addresses {
                ips.insert(addr.addr);
            }
        }
    }

    ips
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_provider_from_host() {
        assert_eq!(
            detect_provider_from_host("api.openai.com"),
            Some(LLMProvider::OpenAI)
        );
        assert_eq!(
            detect_provider_from_host("api.anthropic.com"),
            Some(LLMProvider::Anthropic)
        );
        assert_eq!(
            detect_provider_from_host("api4.cursor.sh"),
            Some(LLMProvider::Cursor)
        );
        assert_eq!(
            detect_provider_from_host("generativelanguage.googleapis.com"),
            Some(LLMProvider::Google)
        );
        assert_eq!(detect_provider_from_host("example.com"), None);
    }

    #[test]
    fn test_sni_extraction_non_tls() {
        let data = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
        assert!(extract_sni_from_packet(data).is_none());
    }

    #[test]
    fn test_sni_extraction_too_short() {
        let data = &[0x16, 0x03, 0x01];
        assert!(extract_sni_from_packet(data).is_none());
    }

    #[test]
    fn test_flow_key_equality() {
        let k1 = FlowKey {
            local_port: 12345,
            remote_ip: IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)),
            remote_port: 443,
        };
        let k2 = FlowKey {
            local_port: 12345,
            remote_ip: IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)),
            remote_port: 443,
        };
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_detect_local_ips_includes_localhost() {
        let ips = detect_local_ips();
        assert!(ips.contains(&IpAddr::V4(Ipv4Addr::LOCALHOST)));
    }

    #[test]
    fn test_monitored_hostnames() {
        let sniffer = PassiveSniffer::new(
            Arc::new(Aggregator::new(
                crate::pricing::PricingDatabase::with_defaults(),
            )),
            Arc::new(TrafficLog::new()),
            None,
        );
        // Exact matches
        assert!(sniffer.is_monitored_hostname("api.openai.com"));
        assert!(sniffer.is_monitored_hostname("api.anthropic.com"));
        // Suffix matches — any *.cursor.sh or *.cursorapi.com
        assert!(sniffer.is_monitored_hostname("api2.cursor.sh"));
        assert!(sniffer.is_monitored_hostname("api4.cursor.sh"));
        assert!(sniffer.is_monitored_hostname("us-east.api5.cursor.sh"));
        assert!(sniffer.is_monitored_hostname("marketplace.cursorapi.com"));
        // Not monitored
        assert!(!sniffer.is_monitored_hostname("example.com"));
        assert!(!sniffer.is_monitored_hostname("cursor.sh")); // no leading dot
        assert!(!sniffer.is_monitored_hostname("evil-cursor.sh"));
    }

    #[test]
    fn test_link_type_from_pcap() {
        assert!(matches!(
            LinkType::from_pcap(pcap::Linktype(1)),
            LinkType::Ethernet
        ));
        assert!(matches!(
            LinkType::from_pcap(pcap::Linktype(0)),
            LinkType::BsdNull
        ));
        assert!(matches!(
            LinkType::from_pcap(pcap::Linktype(12)),
            LinkType::RawIp
        ));
        assert!(matches!(
            LinkType::from_pcap(pcap::Linktype(99)),
            LinkType::Unknown(_)
        ));
    }
}
