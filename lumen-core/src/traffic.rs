use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_TRAFFIC_ENTRIES: usize = 5_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub host: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub request_bytes: u64,
    pub response_bytes: u64,
    pub is_monitored: bool,
    pub data_captured: Vec<String>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAggregate {
    pub host: String,
    pub total_requests: u64,
    pub total_request_bytes: u64,
    pub total_response_bytes: u64,
    pub requests_monitored: u64,
    pub avg_latency_ms: f64,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Default)]
struct HostAccumulator {
    total_requests: u64,
    total_request_bytes: u64,
    total_response_bytes: u64,
    requests_monitored: u64,
    total_latency_ms: u64,
    last_seen: Option<DateTime<Utc>>,
}

pub struct TrafficLog {
    entries: RwLock<VecDeque<TrafficEntry>>,
    host_stats: RwLock<HashMap<String, HostAccumulator>>,
    revision: AtomicU64,
}

impl TrafficLog {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(VecDeque::with_capacity(MAX_TRAFFIC_ENTRIES)),
            host_stats: RwLock::new(HashMap::new()),
            revision: AtomicU64::new(0),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    pub fn record(&self, entry: TrafficEntry) {
        {
            let mut stats = self.host_stats.write();
            let acc = stats.entry(entry.host.clone()).or_default();
            acc.total_requests += 1;
            acc.total_request_bytes += entry.request_bytes;
            acc.total_response_bytes += entry.response_bytes;
            if entry.is_monitored {
                acc.requests_monitored += 1;
            }
            acc.total_latency_ms += entry.latency_ms;
            acc.last_seen = Some(entry.timestamp);
        }

        {
            let mut entries = self.entries.write();
            if entries.len() >= MAX_TRAFFIC_ENTRIES {
                entries.pop_front();
            }
            entries.push_back(entry);
        }

        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn recent(&self, limit: usize) -> Vec<TrafficEntry> {
        let entries = self.entries.read();
        entries.iter().rev().take(limit).cloned().collect()
    }

    pub fn recent_filtered(
        &self,
        limit: usize,
        host_filter: Option<&str>,
        monitored_only: bool,
    ) -> Vec<TrafficEntry> {
        let entries = self.entries.read();
        entries
            .iter()
            .rev()
            .filter(|e| {
                if monitored_only && !e.is_monitored {
                    return false;
                }
                if let Some(h) = host_filter {
                    if !e.host.contains(h) {
                        return false;
                    }
                }
                true
            })
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn host_aggregates(&self) -> Vec<HostAggregate> {
        let stats = self.host_stats.read();
        let mut result: Vec<HostAggregate> = stats
            .iter()
            .map(|(host, acc)| HostAggregate {
                host: host.clone(),
                total_requests: acc.total_requests,
                total_request_bytes: acc.total_request_bytes,
                total_response_bytes: acc.total_response_bytes,
                requests_monitored: acc.requests_monitored,
                avg_latency_ms: if acc.total_requests > 0 {
                    acc.total_latency_ms as f64 / acc.total_requests as f64
                } else {
                    0.0
                },
                last_seen: acc.last_seen.unwrap_or_else(Utc::now),
            })
            .collect();

        result.sort_by(|a, b| b.total_requests.cmp(&a.total_requests));
        result
    }
}

impl Default for TrafficLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(host: &str, monitored: bool) -> TrafficEntry {
        TrafficEntry {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            host: host.to_string(),
            method: "POST".to_string(),
            path: "/v1/chat/completions".to_string(),
            status: 200,
            request_bytes: 1024,
            response_bytes: 2048,
            is_monitored: monitored,
            data_captured: if monitored {
                vec!["tokens_in".into(), "tokens_out".into(), "cost".into()]
            } else {
                vec![]
            },
            latency_ms: 150,
        }
    }

    #[test]
    fn test_record_and_recent() {
        let log = TrafficLog::new();
        log.record(make_entry("api.openai.com", true));
        log.record(make_entry("example.com", false));

        let recent = log.recent(10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].host, "example.com");
        assert_eq!(recent[1].host, "api.openai.com");
    }

    #[test]
    fn test_filter_monitored_only() {
        let log = TrafficLog::new();
        log.record(make_entry("api.openai.com", true));
        log.record(make_entry("example.com", false));
        log.record(make_entry("api.anthropic.com", true));

        let filtered = log.recent_filtered(10, None, true);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|e| e.is_monitored));
    }

    #[test]
    fn test_filter_by_host() {
        let log = TrafficLog::new();
        log.record(make_entry("api.openai.com", true));
        log.record(make_entry("api.anthropic.com", true));
        log.record(make_entry("example.com", false));

        let filtered = log.recent_filtered(10, Some("openai"), false);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].host, "api.openai.com");
    }

    #[test]
    fn test_host_aggregates() {
        let log = TrafficLog::new();
        log.record(make_entry("api.openai.com", true));
        log.record(make_entry("api.openai.com", true));
        log.record(make_entry("example.com", false));

        let aggs = log.host_aggregates();
        assert_eq!(aggs.len(), 2);
        assert_eq!(aggs[0].host, "api.openai.com");
        assert_eq!(aggs[0].total_requests, 2);
        assert_eq!(aggs[0].requests_monitored, 2);
        assert_eq!(aggs[1].host, "example.com");
        assert_eq!(aggs[1].requests_monitored, 0);
    }

    #[test]
    fn test_ring_buffer_eviction() {
        let log = TrafficLog::new();
        for i in 0..MAX_TRAFFIC_ENTRIES + 100 {
            log.record(make_entry(&format!("host-{}", i), false));
        }
        let recent = log.recent(MAX_TRAFFIC_ENTRIES + 200);
        assert_eq!(recent.len(), MAX_TRAFFIC_ENTRIES);
    }
}
