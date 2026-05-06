use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

use crate::parser::{LLMProvider, TokenUsage};
use crate::pricing::{CostBreakdown, PricingDatabase};

const MAX_EVENTS: usize = 10_000;
const ROLLING_WINDOW_SECS: i64 = 3600;
const MAX_LAPS: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub provider: LLMProvider,
    pub model: String,
    pub url: String,
    pub usage: TokenUsage,
    pub cost: CostBreakdown,
    pub lap_number: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateStats {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub total_cache_savings: f64,
    pub event_count: u64,

    pub session_input_tokens: u64, // fresh (non-cached) input
    pub session_output_tokens: u64,
    pub session_cache_read_tokens: u64,
    pub session_cache_creation_tokens: u64,
    pub session_cost: f64,
    pub session_cache_savings: f64,

    pub tokens_per_minute: f64,
    pub cost_per_minute: f64,

    pub top_model: Option<String>,
    pub top_provider: Option<String>,

    pub current_lap: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LapSnapshot {
    pub lap_number: u32,
    pub label: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration_secs: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost: f64,
    pub cache_savings: f64,
    pub event_count: u64,
    pub top_model: Option<String>,
    pub tokens_per_minute: f64,
    pub cost_per_minute: f64,
}

pub struct Aggregator {
    events: RwLock<VecDeque<UsageEvent>>,
    lifetime_stats: RwLock<LifetimeStats>,
    laps: RwLock<Vec<LapSnapshot>>,
    lap_state: RwLock<LapState>,
    pricing: PricingDatabase,
}

#[derive(Debug)]
struct LapState {
    current_lap: u32,
    started_at: DateTime<Utc>,
    event_start_index: usize,
}

impl Default for LapState {
    fn default() -> Self {
        Self {
            current_lap: 1,
            started_at: Utc::now(),
            event_start_index: 0,
        }
    }
}

#[derive(Debug, Default)]
struct LifetimeStats {
    total_input: u64,
    total_output: u64,
    total_cost: f64,
    total_cache_savings: f64,
    event_count: u64,
    model_counts: HashMap<String, u64>,
    provider_counts: HashMap<String, u64>,
}

impl Aggregator {
    pub fn new(pricing: PricingDatabase) -> Self {
        Self {
            events: RwLock::new(VecDeque::with_capacity(MAX_EVENTS)),
            lifetime_stats: RwLock::new(LifetimeStats::default()),
            laps: RwLock::new(Vec::new()),
            lap_state: RwLock::new(LapState::default()),
            pricing,
        }
    }

    pub fn record_usage(&self, provider: LLMProvider, model: &str, url: &str, usage: TokenUsage) {
        let cost = self.pricing.calculate_cost(
            provider,
            model,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_creation_tokens,
        );

        // Full billed input: Anthropic reports fresh input, cache_read, and cache_creation
        // as separate fields. Aggregate all three so token counts reflect reality.
        let full_input = usage.input_tokens
            + usage.cache_read_tokens.unwrap_or(0)
            + usage.cache_creation_tokens.unwrap_or(0);

        let lap_number = self.lap_state.read().current_lap;
        let event = UsageEvent {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            provider,
            model: model.to_string(),
            url: url.to_string(),
            usage,
            cost,
            lap_number,
        };

        {
            let mut stats = self.lifetime_stats.write();
            stats.total_input += full_input;
            stats.total_output += event.usage.output_tokens;
            stats.total_cost += event.cost.total_cost;
            stats.total_cache_savings += event.cost.cache_read_savings;
            stats.event_count += 1;
            *stats.model_counts.entry(event.model.clone()).or_insert(0) += 1;
            *stats
                .provider_counts
                .entry(event.cost.provider.clone())
                .or_insert(0) += 1;
        }

        {
            let mut events = self.events.write();
            if events.len() >= MAX_EVENTS {
                events.pop_front();
                let mut lap = self.lap_state.write();
                if lap.event_start_index > 0 {
                    lap.event_start_index -= 1;
                }
            }
            events.push_back(event);
        }
    }

    pub fn compute_stats(&self) -> AggregateStats {
        let lifetime = self.lifetime_stats.read();
        let events = self.events.read();
        let lap = self.lap_state.read();

        let now = Utc::now();
        let window_start = now - chrono::Duration::seconds(ROLLING_WINDOW_SECS);

        let mut session_input = 0u64;
        let mut session_output = 0u64;
        let mut session_cache_read = 0u64;
        let mut session_cache_create = 0u64;
        let mut session_cost = 0.0f64;
        let mut session_cache_savings = 0.0f64;
        let mut window_tokens = 0u64;
        let mut window_cost = 0.0f64;
        let mut window_count = 0u64;

        for event in events.iter().skip(lap.event_start_index) {
            let cr = event.usage.cache_read_tokens.unwrap_or(0);
            let cc = event.usage.cache_creation_tokens.unwrap_or(0);
            session_input += event.usage.input_tokens;
            session_output += event.usage.output_tokens;
            session_cache_read += cr;
            session_cache_create += cc;
            session_cost += event.cost.total_cost;
            session_cache_savings += event.cost.cache_read_savings;

            if event.timestamp >= window_start {
                window_tokens += event.usage.input_tokens + cr + cc + event.usage.output_tokens;
                window_cost += event.cost.total_cost;
                window_count += 1;
            }
        }

        let elapsed_mins = if window_count > 0 {
            events
                .iter()
                .skip(lap.event_start_index)
                .find(|e| e.timestamp >= window_start)
                .map(|e| (now - e.timestamp).num_seconds() as f64 / 60.0)
                .unwrap_or(1.0)
                .max(1.0)
        } else {
            1.0
        };

        let top_model = lifetime
            .model_counts
            .iter()
            .max_by_key(|(_, c)| *c)
            .map(|(m, _)| m.clone());

        let top_provider = lifetime
            .provider_counts
            .iter()
            .max_by_key(|(_, c)| *c)
            .map(|(p, _)| p.clone());

        AggregateStats {
            total_input_tokens: lifetime.total_input,
            total_output_tokens: lifetime.total_output,
            total_tokens: lifetime.total_input + lifetime.total_output,
            total_cost: lifetime.total_cost,
            total_cache_savings: lifetime.total_cache_savings,
            event_count: lifetime.event_count,
            session_input_tokens: session_input,
            session_output_tokens: session_output,
            session_cache_read_tokens: session_cache_read,
            session_cache_creation_tokens: session_cache_create,
            session_cost,
            session_cache_savings,
            tokens_per_minute: window_tokens as f64 / elapsed_mins,
            cost_per_minute: window_cost / elapsed_mins,
            top_model,
            top_provider,
            current_lap: lap.current_lap,
        }
    }

    pub fn create_lap(&self, label: Option<String>) -> LapSnapshot {
        let events = self.events.read();
        let mut lap = self.lap_state.write();
        let now = Utc::now();

        let mut input = 0u64;
        let mut output = 0u64;
        let mut cost = 0.0f64;
        let mut cache_savings = 0.0f64;
        let mut count = 0u64;
        let mut model_counts: HashMap<String, u64> = HashMap::new();

        for event in events.iter().skip(lap.event_start_index) {
            input += event.usage.input_tokens
                + event.usage.cache_read_tokens.unwrap_or(0)
                + event.usage.cache_creation_tokens.unwrap_or(0);
            output += event.usage.output_tokens;
            cost += event.cost.total_cost;
            cache_savings += event.cost.cache_read_savings;
            count += 1;
            *model_counts.entry(event.model.clone()).or_insert(0) += 1;
        }

        let duration_secs = (now - lap.started_at).num_milliseconds() as f64 / 1000.0;
        let duration_mins = (duration_secs / 60.0).max(1.0);

        let top_model = model_counts
            .iter()
            .max_by_key(|(_, c)| *c)
            .map(|(m, _)| m.clone());

        let default_label = format!("Lap {}", lap.current_lap);
        let snapshot = LapSnapshot {
            lap_number: lap.current_lap,
            label: label.unwrap_or(default_label),
            started_at: lap.started_at,
            ended_at: now,
            duration_secs,
            input_tokens: input,
            output_tokens: output,
            total_tokens: input + output,
            cost,
            cache_savings,
            event_count: count,
            top_model,
            tokens_per_minute: (input + output) as f64 / duration_mins,
            cost_per_minute: cost / duration_mins,
        };

        lap.current_lap += 1;
        lap.started_at = now;
        lap.event_start_index = events.len();

        drop(lap);
        drop(events);

        let mut laps = self.laps.write();
        if laps.len() >= MAX_LAPS {
            laps.remove(0);
        }
        laps.push(snapshot.clone());

        snapshot
    }

    pub fn get_laps(&self) -> Vec<LapSnapshot> {
        self.laps.read().clone()
    }

    pub fn recent_events(&self, limit: usize) -> Vec<UsageEvent> {
        let events = self.events.read();
        events.iter().rev().take(limit).cloned().collect()
    }

    pub fn clear(&self) {
        self.events.write().clear();
        *self.lifetime_stats.write() = LifetimeStats::default();
        self.laps.write().clear();
        *self.lap_state.write() = LapState::default();
    }
}

impl Default for Aggregator {
    fn default() -> Self {
        Self::new(PricingDatabase::with_defaults())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::LLMProvider;

    fn record_n(agg: &Aggregator, n: usize, model: &str) {
        for _ in 0..n {
            let usage = TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                total_tokens: 150,
                cache_read_tokens: None,
                cache_creation_tokens: None,
            };
            agg.record_usage(
                LLMProvider::OpenAI,
                model,
                "https://api.openai.com/v1/chat/completions",
                usage,
            );
        }
    }

    #[test]
    fn test_record_and_stats() {
        let agg = Aggregator::default();
        record_n(&agg, 3, "gpt-4o");

        let stats = agg.compute_stats();
        assert_eq!(stats.event_count, 3);
        assert_eq!(stats.total_input_tokens, 300);
        assert_eq!(stats.total_output_tokens, 150);
        assert_eq!(stats.total_tokens, 450);
        assert!(stats.total_cost > 0.0);
        assert_eq!(stats.top_model.as_deref(), Some("gpt-4o"));
        assert_eq!(stats.current_lap, 1);
    }

    #[test]
    fn test_session_tokens_scoped_to_lap() {
        let agg = Aggregator::default();
        record_n(&agg, 5, "gpt-4o");

        let stats = agg.compute_stats();
        assert_eq!(stats.session_input_tokens, 500);

        agg.create_lap(None);

        let stats_after = agg.compute_stats();
        assert_eq!(stats_after.session_input_tokens, 0);
        assert_eq!(stats_after.current_lap, 2);
        assert_eq!(stats_after.total_input_tokens, 500);
    }

    #[test]
    fn test_lap_snapshot_content() {
        let agg = Aggregator::default();
        record_n(&agg, 4, "gpt-4o");

        let snap = agg.create_lap(Some("test lap".to_string()));
        assert_eq!(snap.lap_number, 1);
        assert_eq!(snap.label, "test lap");
        assert_eq!(snap.event_count, 4);
        assert_eq!(snap.input_tokens, 400);
        assert_eq!(snap.output_tokens, 200);
        assert_eq!(snap.total_tokens, 600);
        assert!(snap.cost > 0.0);
        assert!(snap.duration_secs >= 0.0);
        assert_eq!(snap.top_model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn test_default_lap_label() {
        let agg = Aggregator::default();
        record_n(&agg, 1, "gpt-4o");

        let snap = agg.create_lap(None);
        assert_eq!(snap.label, "Lap 1");
    }

    #[test]
    fn test_multiple_laps() {
        let agg = Aggregator::default();

        record_n(&agg, 2, "gpt-4o");
        agg.create_lap(Some("first".to_string()));

        record_n(&agg, 3, "gpt-4o");
        agg.create_lap(Some("second".to_string()));

        record_n(&agg, 1, "gpt-4o");

        let laps = agg.get_laps();
        assert_eq!(laps.len(), 2);
        assert_eq!(laps[0].lap_number, 1);
        assert_eq!(laps[0].event_count, 2);
        assert_eq!(laps[1].lap_number, 2);
        assert_eq!(laps[1].event_count, 3);

        let stats = agg.compute_stats();
        assert_eq!(stats.current_lap, 3);
        assert_eq!(stats.session_input_tokens, 100);
        assert_eq!(stats.total_input_tokens, 600);
    }

    #[test]
    fn test_empty_lap() {
        let agg = Aggregator::default();
        let snap = agg.create_lap(None);
        assert_eq!(snap.event_count, 0);
        assert_eq!(snap.total_tokens, 0);
        assert_eq!(snap.cost, 0.0);
    }

    #[test]
    fn test_clear_resets_laps() {
        let agg = Aggregator::default();
        record_n(&agg, 3, "gpt-4o");
        agg.create_lap(None);
        record_n(&agg, 2, "gpt-4o");

        agg.clear();

        let stats = agg.compute_stats();
        assert_eq!(stats.event_count, 0);
        assert_eq!(stats.current_lap, 1);
        assert_eq!(stats.session_input_tokens, 0);
        assert!(agg.get_laps().is_empty());
    }

    #[test]
    fn test_recent_events() {
        let agg = Aggregator::default();
        record_n(&agg, 10, "gpt-4o");

        let recent = agg.recent_events(3);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_lap_preserves_lifetime_totals() {
        let agg = Aggregator::default();
        record_n(&agg, 5, "gpt-4o");
        let cost_before = agg.compute_stats().total_cost;

        agg.create_lap(None);

        let stats = agg.compute_stats();
        assert_eq!(stats.total_cost, cost_before);
        assert_eq!(stats.total_input_tokens, 500);
    }
}
