pub mod loader;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::parser::LLMProvider;

// ---------------------------------------------------------------------------
// JSON file schema — loaded at startup from ~/.lumen/pricing.json or remote.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PricingFile {
    pub schema_version: u32,
    pub updated: String,
    pub models: Vec<PricingEntry>,
}

#[derive(Debug, Deserialize)]
pub struct PricingEntry {
    pub provider: String,
    pub model: String,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: Option<f64>,
    pub cache_write_per_mtok: Option<f64>,
    // Informational only — key into the top-level "notes" map if present.
    #[allow(dead_code)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub provider: String,
    pub model: String,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: Option<f64>,
    pub cache_write_per_mtok: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub input_cost: f64,
    pub output_cost: f64,
    pub cache_read_savings: f64,
    pub total_cost: f64,
    pub model: String,
    pub provider: String,
}

pub struct PricingDatabase {
    models: HashMap<String, ModelPricing>,
}

impl PricingDatabase {
    pub fn with_defaults() -> Self {
        let mut db = Self {
            models: HashMap::new(),
        };

        // GPT-5.x family (released 2025-2026)
        // All use 10% cached-input rate. -pro variants must be explicit so they
        // aren't fuzzy-matched down to the base model's much cheaper rate.
        db.add("openai", "gpt-5.5",          5.00,  30.00, Some(0.50),  None);
        db.add("openai", "gpt-5.5-pro",     30.00, 180.00, None,         None);
        db.add("openai", "gpt-5.4",          2.50,  15.00, Some(0.25),  None);
        db.add("openai", "gpt-5.4-mini",     0.75,   4.50, Some(0.075), None);
        db.add("openai", "gpt-5.4-nano",     0.20,   1.25, Some(0.02),  None);
        db.add("openai", "gpt-5.4-pro",     30.00, 180.00, None,         None);
        db.add("openai", "gpt-5.3-codex",    1.75,  14.00, Some(0.175), None);
        db.add("openai", "gpt-5.1-codex-mini", 0.25, 2.00, Some(0.025), None);

        db.add("openai", "gpt-4o", 2.50, 10.00, Some(1.25), None);
        // Dated snapshot with different (higher) rate — must be explicit so it isn't
        // fuzzy-matched to the current gpt-4o price.
        db.add("openai", "gpt-4o-2024-05-13", 5.00, 15.00, None, None);
        db.add("openai", "chatgpt-4o-latest", 5.00, 15.00, None, None);
        db.add("openai", "gpt-4o-mini", 0.15, 0.60, Some(0.075), None);
        db.add("openai", "gpt-4.1", 2.00, 8.00, Some(0.50), None);
        db.add("openai", "gpt-4.1-mini", 0.40, 1.60, Some(0.10), None);
        db.add("openai", "gpt-4.1-nano", 0.10, 0.40, Some(0.025), None);
        db.add("openai", "gpt-4-turbo", 10.00, 30.00, None, None);
        db.add("openai", "gpt-4", 30.00, 60.00, None, None);
        db.add("openai", "gpt-3.5-turbo", 0.50, 1.50, None, None);
        db.add("openai", "o1", 15.00, 60.00, Some(7.50), None);
        db.add("openai", "o1-pro", 150.00, 600.00, None, None);
        db.add("openai", "o1-mini", 3.00, 12.00, Some(1.50), None);
        db.add("openai", "o1-preview", 15.00, 60.00, None, None);
        db.add("openai", "o3", 2.00, 8.00, Some(0.50), None);
        db.add("openai", "o3-pro", 20.00, 80.00, None, None);
        db.add("openai", "o3-mini", 1.10, 4.40, Some(0.55), None);
        db.add("openai", "o4-mini", 1.10, 4.40, Some(0.275), None);
        db.add("openai", "codex-mini-latest", 1.50, 6.00, Some(0.375), None);

        // Deprecated Opus 4 / Opus 4.1 — $15/$75 tier
        db.add(
            "anthropic",
            "claude-opus-4-20250514",
            15.00,
            75.00,
            Some(1.50),
            Some(18.75),
        );
        db.add(
            "anthropic",
            "claude-opus-4-0",
            15.00,
            75.00,
            Some(1.50),
            Some(18.75),
        );
        db.add(
            "anthropic",
            "claude-opus-4-1",
            15.00,
            75.00,
            Some(1.50),
            Some(18.75),
        );
        db.add(
            "anthropic",
            "claude-opus-4-1-20250805",
            15.00,
            75.00,
            Some(1.50),
            Some(18.75),
        );

        // Opus 4.5 / 4.6 / 4.7 — $5/$25 tier
        db.add(
            "anthropic",
            "claude-opus-4-5",
            5.00,
            25.00,
            Some(0.50),
            Some(6.25),
        );
        db.add(
            "anthropic",
            "claude-opus-4-5-20251101",
            5.00,
            25.00,
            Some(0.50),
            Some(6.25),
        );
        db.add(
            "anthropic",
            "claude-opus-4-6",
            5.00,
            25.00,
            Some(0.50),
            Some(6.25),
        );
        db.add(
            "anthropic",
            "claude-opus-4-7",
            5.00,
            25.00,
            Some(0.50),
            Some(6.25),
        );

        // Sonnet 4 family — $3/$15 tier
        db.add(
            "anthropic",
            "claude-sonnet-4-20250514",
            3.00,
            15.00,
            Some(0.30),
            Some(3.75),
        );
        db.add(
            "anthropic",
            "claude-sonnet-4-0",
            3.00,
            15.00,
            Some(0.30),
            Some(3.75),
        );
        db.add(
            "anthropic",
            "claude-sonnet-4-5",
            3.00,
            15.00,
            Some(0.30),
            Some(3.75),
        );
        db.add(
            "anthropic",
            "claude-sonnet-4-5-20250929",
            3.00,
            15.00,
            Some(0.30),
            Some(3.75),
        );
        db.add(
            "anthropic",
            "claude-sonnet-4-6",
            3.00,
            15.00,
            Some(0.30),
            Some(3.75),
        );
        db.add(
            "anthropic",
            "claude-3-5-sonnet-20241022",
            3.00,
            15.00,
            Some(0.30),
            Some(3.75),
        );

        // Haiku 4.5 — $1/$5 tier
        db.add(
            "anthropic",
            "claude-haiku-4-5",
            1.00,
            5.00,
            Some(0.10),
            Some(1.25),
        );
        db.add(
            "anthropic",
            "claude-haiku-4-5-20251001",
            1.00,
            5.00,
            Some(0.10),
            Some(1.25),
        );

        // Legacy Haiku 3.5 (retired except Bedrock/Vertex)
        db.add(
            "anthropic",
            "claude-3-5-haiku-20241022",
            0.80,
            4.00,
            Some(0.08),
            Some(1.00),
        );
        db.add(
            "anthropic",
            "claude-3-haiku-20240307",
            0.25,
            1.25,
            Some(0.03),
            Some(0.30),
        );

        db.add("google", "gemini-2.5-pro", 1.25, 10.00, None, None);
        db.add("google", "gemini-2.5-flash", 0.15, 0.60, None, None);
        db.add("google", "gemini-2.0-flash", 0.10, 0.40, None, None);
        db.add("google", "gemini-1.5-pro", 1.25, 5.00, None, None);
        db.add("google", "gemini-1.5-flash", 0.075, 0.30, None, None);

        // ----------------------------------------------------------------------
        // Cursor pricing
        //
        // Cursor's "On-Demand" billing is dramatically lower than the underlying
        // Anthropic / OpenAI direct API rates because:
        //
        //   1. Cursor caches conversation context very aggressively. Cache hit
        //      rates >85% are typical for follow-up turns in a session.
        //   2. The cache is invisible to the proxy — Cursor's binary protobuf
        //      response doesn't expose `cache_read_input_tokens` the way
        //      Anthropic's JSON API does. We can't separate cached from fresh
        //      input.
        //   3. Cursor sends the conversation context out-of-band via background
        //      `BidiAppend` calls (which are filtered as noise) and the actual
        //      `RunSSE` request only carries the user delta. So byte-estimation
        //      on the visible request body massively undercounts input tokens
        //      and most byte-estimated tokens land in `output_tokens`.
        //
        // The net effect is that pricing Cursor traffic at Anthropic-direct
        // rates (e.g. $25/MTok for Opus 4.7 output) overshoots real invoice
        // numbers by 5–10x. The rates below are calibrated against observed
        // Cursor "On-Demand" invoices (e.g. claude-opus-4-7-thinking-high
        // billed at ~$2.84/MTok blended on a 963K-token call) so that Lumen's
        // cost numbers land within ~2x of the actual Cursor bill.
        //
        // These are EFFECTIVE blended rates that absorb byte-estimation bias.
        // If we ever start extracting real cache-aware usage from Cursor's
        // gRPC responses, these should be revisited (and likely raised toward
        // the underlying Anthropic/OpenAI direct rates).
        // ----------------------------------------------------------------------
        db.add("cursor", "claude-opus-4-7", 3.00, 5.00, Some(0.30), Some(3.75));
        db.add("cursor", "claude-opus-4", 3.00, 5.00, Some(0.30), Some(3.75));
        db.add("cursor", "claude-sonnet-4-6", 0.60, 1.00, Some(0.06), Some(0.75));
        db.add("cursor", "claude-sonnet-4", 0.60, 1.00, Some(0.06), Some(0.75));
        db.add("cursor", "claude-haiku-4-5", 0.16, 0.27, Some(0.016), Some(0.20));
        db.add("cursor", "claude-haiku-4", 0.16, 0.27, Some(0.016), Some(0.20));

        // Cursor's own first-party models (Composer family) — flat, very cheap.
        // composer-2-fast invoices around $0.75/MTok blended.
        db.add("cursor", "composer-2", 0.10, 0.50, None, None);
        db.add("cursor", "composer-2-fast", 0.10, 0.50, None, None);

        // OpenAI models routed through Cursor — calibrated similarly.
        db.add("cursor", "gpt-5.5", 0.20, 1.00, Some(0.02), None);
        db.add("cursor", "gpt-5", 0.30, 1.50, Some(0.03), None);
        db.add("cursor", "gpt-5-mini", 0.06, 0.30, Some(0.006), None);
        db.add("cursor", "gpt-5-nano", 0.02, 0.10, Some(0.002), None);

        // Cursor fallback — used when we can't detect the specific model.
        // Conservative blended rate; far lower than the Anthropic-direct $3/$15
        // we used previously, which overshot real Cursor invoices badly.
        db.add("cursor", "cursor-unknown", 0.50, 2.50, None, None);

        db
    }

    /// Build a database from a parsed JSON pricing file.
    /// Falls back to `with_defaults()` and logs a warning if schema_version is unsupported.
    pub fn from_file(file: &PricingFile) -> Self {
        if file.schema_version != 1 {
            tracing::warn!(
                "Unsupported pricing schema_version {}; falling back to compiled-in defaults",
                file.schema_version
            );
            return Self::with_defaults();
        }
        let mut db = Self {
            models: HashMap::new(),
        };
        for e in &file.models {
            db.add(
                &e.provider,
                &e.model,
                e.input_per_mtok,
                e.output_per_mtok,
                e.cache_read_per_mtok,
                e.cache_write_per_mtok,
            );
        }
        tracing::info!(
            "Loaded {} model pricing entries from JSON (updated {})",
            db.models.len(),
            file.updated
        );
        db
    }

    fn add(
        &mut self,
        provider: &str,
        model: &str,
        input: f64,
        output: f64,
        cache_read: Option<f64>,
        cache_write: Option<f64>,
    ) {
        let key = format!("{}:{}", provider, model);
        self.models.insert(
            key,
            ModelPricing {
                provider: provider.to_string(),
                model: model.to_string(),
                input_per_mtok: input,
                output_per_mtok: output,
                cache_read_per_mtok: cache_read,
                cache_write_per_mtok: cache_write,
            },
        );
    }

    pub fn calculate_cost(
        &self,
        provider: LLMProvider,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: Option<u64>,
        cache_creation_tokens: Option<u64>,
    ) -> CostBreakdown {
        let provider_str = provider.to_string();

        let pricing = if provider == LLMProvider::Cursor {
            // Cursor lookup precedence:
            //   1. exact match in the cursor namespace (cursor:<model>)
            //   2. fuzzy match within the cursor namespace — this is critical
            //      so variants like `claude-opus-4-7-thinking-high` resolve to
            //      `cursor:claude-opus-4-7` instead of falling through to the
            //      Anthropic-direct `claude-opus-4-7` entry, which is priced
            //      at $5/$25 and overshoots real Cursor billing by 5–10x.
            //   3. cross-provider match — a true fallback used only for
            //      cursor-routed models we haven't explicitly catalogued.
            //   4. cursor-unknown sentinel (cheap blended fallback).
            let cursor_key = format!("cursor:{}", model);
            self.models
                .get(&cursor_key)
                .or_else(|| self.fuzzy_match("cursor", model))
                .or_else(|| self.cross_provider_match(model))
                .or_else(|| self.models.get("cursor:cursor-unknown"))
        } else {
            let key = format!("{}:{}", provider_str, model);
            self.models
                .get(&key)
                .or_else(|| self.fuzzy_match(&provider_str, model))
        };

        if let Some(pricing) = pricing {
            let cache_read = cache_read_tokens.unwrap_or(0);
            let cache_create = cache_creation_tokens.unwrap_or(0);
            // Normal input excludes cache-read and cache-creation tokens (billed separately)
            let normal_input = input_tokens
                .saturating_sub(cache_read)
                .saturating_sub(cache_create);

            let input_cost = (normal_input as f64 / 1_000_000.0) * pricing.input_per_mtok;
            let output_cost = (output_tokens as f64 / 1_000_000.0) * pricing.output_per_mtok;

            let cache_read_cost = if let Some(rate) = pricing.cache_read_per_mtok {
                (cache_read as f64 / 1_000_000.0) * rate
            } else {
                (cache_read as f64 / 1_000_000.0) * pricing.input_per_mtok
            };

            // Cache creation costs 25% more than base input (or use explicit rate)
            let cache_create_cost = if let Some(rate) = pricing.cache_write_per_mtok {
                (cache_create as f64 / 1_000_000.0) * rate
            } else {
                (cache_create as f64 / 1_000_000.0) * pricing.input_per_mtok * 1.25
            };

            // Savings = what cache_read tokens would have cost at full rate minus what they actually cost
            let cache_read_savings = (cache_read as f64 / 1_000_000.0)
                * (pricing.input_per_mtok
                    - pricing
                        .cache_read_per_mtok
                        .unwrap_or(pricing.input_per_mtok));

            CostBreakdown {
                input_cost: input_cost + cache_read_cost + cache_create_cost,
                output_cost,
                cache_read_savings: cache_read_savings.max(0.0),
                total_cost: input_cost + cache_read_cost + cache_create_cost + output_cost,
                model: model.to_string(),
                provider: provider_str,
            }
        } else {
            let fallback_rate = 5.0;
            let input_cost = (input_tokens as f64 / 1_000_000.0) * fallback_rate;
            let output_cost = (output_tokens as f64 / 1_000_000.0) * fallback_rate * 3.0;

            CostBreakdown {
                input_cost,
                output_cost,
                cache_read_savings: 0.0,
                total_cost: input_cost + output_cost,
                model: model.to_string(),
                provider: provider_str,
            }
        }
    }

    fn fuzzy_match(&self, provider: &str, model: &str) -> Option<&ModelPricing> {
        self.models
            .iter()
            .filter(|(k, _)| k.starts_with(&format!("{}:", provider)))
            .filter(|(_, v)| {
                // Accept a prefix match only when the character immediately after
                // the shared prefix is '-'.  This lets "gpt-4o-2024-08-06" fuzzy-
                // match "gpt-4o" and "claude-opus-4-7-thinking-high" match
                // "claude-opus-4-7", while preventing "gpt-4.1" from accidentally
                // resolving to "gpt-4" (different model family, different pricing).
                let (a, b) = (model, v.model.as_str());
                let suffix_is_dash = |s: &str, prefix: &str| {
                    s.starts_with(prefix) && s.as_bytes().get(prefix.len()) == Some(&b'-')
                };
                suffix_is_dash(a, b) || suffix_is_dash(b, a)
            })
            .max_by_key(|(_, v)| v.model.len())
            .map(|(_, v)| v)
    }

    /// For proxy providers like Cursor: search all providers for a matching model.
    fn cross_provider_match(&self, model: &str) -> Option<&ModelPricing> {
        for provider in ["anthropic", "openai", "google"] {
            let key = format!("{}:{}", provider, model);
            if let Some(p) = self.models.get(&key) {
                return Some(p);
            }
            if let Some(p) = self.fuzzy_match(provider, model) {
                return Some(p);
            }
        }
        None
    }
}

impl Default for PricingDatabase {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_model_pricing() {
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(
            LLMProvider::OpenAI,
            "gpt-4o",
            1_000_000,
            500_000,
            None,
            None,
        );
        assert!((cost.input_cost - 2.50).abs() < 0.01);
        assert!((cost.output_cost - 5.00).abs() < 0.01);
    }

    #[test]
    fn test_cache_savings() {
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(
            LLMProvider::Anthropic,
            "claude-3-5-sonnet-20241022",
            100_000,
            10_000,
            Some(80_000),
            None,
        );
        assert!(cost.cache_read_savings > 0.0);
    }

    #[test]
    fn test_fuzzy_match() {
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(
            LLMProvider::OpenAI,
            "gpt-4o-2024-08-06",
            1_000_000,
            500_000,
            None,
            None,
        );
        assert!((cost.input_cost - 2.50).abs() < 0.01);
    }

    #[test]
    fn test_unknown_model_fallback() {
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(
            LLMProvider::OpenAI,
            "gpt-99-turbo",
            1_000_000,
            500_000,
            None,
            None,
        );
        assert!(cost.total_cost > 0.0);
    }

    #[test]
    fn test_cursor_uses_cursor_pricing_not_anthropic_direct() {
        // When a Claude model is routed through Cursor, we MUST price it at the
        // calibrated cursor:* rates, not at Anthropic-direct API rates. Anthropic
        // direct rates for Sonnet are $3/$15 — pricing Cursor traffic that way
        // overshoots real Cursor invoices by ~15x.
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(
            LLMProvider::Cursor,
            "claude-sonnet-4-20250514",
            1_000_000,
            500_000,
            None,
            None,
        );
        // cursor:claude-sonnet-4 = $0.60 input / $1.00 output
        assert!(
            (cost.input_cost - 0.60).abs() < 0.01,
            "got input_cost {}",
            cost.input_cost
        );
        assert!(
            (cost.output_cost - 0.50).abs() < 0.01,
            "got output_cost {}",
            cost.output_cost
        );
        assert_eq!(cost.provider, "cursor");
    }

    #[test]
    fn test_cursor_opus_thinking_variant_resolves_to_cursor_opus() {
        // Cursor exposes Opus 4.7 as "claude-opus-4-7-thinking-high" in their
        // billing UI. Lumen extracts the model as either the full string or the
        // shorter "claude-opus-4-7" — both must resolve to cursor:claude-opus-4-7
        // pricing, NOT to anthropic:claude-opus-4-20250514 ($15/$75).
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(
            LLMProvider::Cursor,
            "claude-opus-4-7-thinking-high",
            1_000_000,
            500_000,
            None,
            None,
        );
        // cursor:claude-opus-4-7 = $3.00 input / $5.00 output
        assert!(
            (cost.input_cost - 3.00).abs() < 0.01,
            "expected $3.00 input, got {}",
            cost.input_cost
        );
        assert!(
            (cost.output_cost - 2.50).abs() < 0.01,
            "expected $2.50 output, got {}",
            cost.output_cost
        );
        assert_eq!(cost.provider, "cursor");
    }

    #[test]
    fn test_cursor_composer_priced() {
        // composer-2-fast invoices around $0.75/MTok blended on real Cursor bills.
        // Without an explicit cursor:composer-2-fast entry it would fall through
        // to the $5/$15 unknown-model fallback (way too high).
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(
            LLMProvider::Cursor,
            "composer-2-fast",
            500_000,
            500_000,
            None,
            None,
        );
        // 0.5M * $0.10 + 0.5M * $0.50 = $0.05 + $0.25 = $0.30
        assert!(
            (cost.total_cost - 0.30).abs() < 0.01,
            "expected $0.30, got {}",
            cost.total_cost
        );
    }

    #[test]
    fn test_cursor_opus_realistic_call_against_invoice() {
        // Calibration check against a real Cursor invoice line:
        //   claude-opus-4-7-thinking-high, 963.1K tokens, $2.74
        // We don't know Cursor's input/output split, but with byte-estimation
        // most tokens land in output_tokens. Worst case (all output) at
        // cursor:claude-opus-4-7's $5/MTok output rate:
        //   963_100 * $5/MTok = $4.82
        // That's within ~2x of the $2.74 invoice — vs the $24.08 we'd compute
        // with the Anthropic-direct $25/MTok rate.
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(LLMProvider::Cursor, "claude-opus-4-7", 0, 963_100, None, None);
        assert!(
            cost.total_cost < 6.0,
            "cost should be in the ~$2-5 ballpark, got {}",
            cost.total_cost
        );
        assert!(
            cost.total_cost > 1.0,
            "cost should not be near zero, got {}",
            cost.total_cost
        );
    }

    #[test]
    fn test_cursor_fuzzy_cross_provider() {
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(
            LLMProvider::Cursor,
            "gpt-4o-2024-08-06",
            1_000_000,
            500_000,
            None,
            None,
        );
        assert!((cost.input_cost - 2.50).abs() < 0.01);
    }

    #[test]
    fn test_cursor_unknown_model_fallback() {
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(
            LLMProvider::Cursor,
            "cursor-internal-model",
            1_000_000,
            500_000,
            None,
            None,
        );
        // Should use fallback pricing
        assert!(cost.total_cost > 0.0);
    }

    #[test]
    fn test_cache_creation_billed() {
        let db = PricingDatabase::with_defaults();
        // 100k input, 80k of which are cache writes, no cache reads
        // Normal input: 20k * $3/MTok = $0.06
        // Cache create: 80k * $3.75/MTok = $0.30
        // Total input side: $0.36
        let cost = db.calculate_cost(
            LLMProvider::Anthropic,
            "claude-sonnet-4-20250514",
            100_000,
            0,
            None,
            Some(80_000),
        );
        assert!(
            (cost.input_cost - 0.36).abs() < 0.001,
            "got {}",
            cost.input_cost
        );
        assert!((cost.total_cost - 0.36).abs() < 0.001);
    }

    #[test]
    fn test_claude_sonnet_4_6_alias() {
        let db = PricingDatabase::with_defaults();
        // claude-sonnet-4-6 should resolve to same pricing as claude-sonnet-4-20250514
        let cost = db.calculate_cost(
            LLMProvider::Anthropic,
            "claude-sonnet-4-6",
            1_000_000,
            500_000,
            None,
            None,
        );
        assert!(
            (cost.input_cost - 3.00).abs() < 0.01,
            "got {}",
            cost.input_cost
        );
        assert!((cost.output_cost - 7.50).abs() < 0.01);
    }

    #[test]
    fn test_cache_read_savings_calculation() {
        let db = PricingDatabase::with_defaults();
        // 100k input, 80k cache reads → savings = 80k * ($3.00 - $0.30) = $0.216
        let cost = db.calculate_cost(
            LLMProvider::Anthropic,
            "claude-sonnet-4-6",
            100_000,
            0,
            Some(80_000),
            None,
        );
        assert!(
            (cost.cache_read_savings - 0.216).abs() < 0.001,
            "got {}",
            cost.cache_read_savings
        );
    }

    // -----------------------------------------------------------------------
    // 0.1.5 — corrected model pricing
    // -----------------------------------------------------------------------

    #[test]
    fn test_opus_4_7_is_5_25_not_15_75() {
        // claude-opus-4-7 is the $5/$25 tier (same as Opus 4.5/4.6), NOT the
        // old deprecated Opus 4 tier ($15/$75). Getting this wrong causes costs
        // to be reported 3x too high.
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(
            LLMProvider::Anthropic,
            "claude-opus-4-7",
            1_000_000,
            1_000_000,
            None,
            None,
        );
        assert!(
            (cost.input_cost - 5.00).abs() < 0.01,
            "expected $5.00 input (not $15), got {}",
            cost.input_cost
        );
        assert!(
            (cost.output_cost - 25.00).abs() < 0.01,
            "expected $25.00 output (not $75), got {}",
            cost.output_cost
        );
        // Cache read should be $0.50/MTok, not $1.50
        let cost_cached = db.calculate_cost(
            LLMProvider::Anthropic,
            "claude-opus-4-7",
            1_000_000,
            0,
            Some(1_000_000),
            None,
        );
        assert!(
            (cost_cached.total_cost - 0.50).abs() < 0.01,
            "expected $0.50 cache read (not $1.50), got {}",
            cost_cached.total_cost
        );
    }

    #[test]
    fn test_haiku_4_5_is_1_5_not_0_80_4() {
        // claude-haiku-4-5 is the $1/$5 tier, not the retired Haiku 3.5 rate.
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(
            LLMProvider::Anthropic,
            "claude-haiku-4-5",
            1_000_000,
            1_000_000,
            None,
            None,
        );
        assert!(
            (cost.input_cost - 1.00).abs() < 0.01,
            "expected $1.00 input, got {}",
            cost.input_cost
        );
        assert!(
            (cost.output_cost - 5.00).abs() < 0.01,
            "expected $5.00 output, got {}",
            cost.output_cost
        );
    }

    #[test]
    fn test_gpt_4_1_family_priced() {
        let db = PricingDatabase::with_defaults();
        for (model, exp_in, exp_out) in [
            ("gpt-4.1",      2.00_f64, 8.00_f64),
            ("gpt-4.1-mini", 0.40,     1.60),
            ("gpt-4.1-nano", 0.10,     0.40),
        ] {
            let cost = db.calculate_cost(LLMProvider::OpenAI, model, 1_000_000, 1_000_000, None, None);
            assert!(
                (cost.input_cost - exp_in).abs() < 0.001,
                "{}: expected ${} input, got {}",
                model, exp_in, cost.input_cost
            );
            assert!(
                (cost.output_cost - exp_out).abs() < 0.001,
                "{}: expected ${} output, got {}",
                model, exp_out, cost.output_cost
            );
        }
    }

    // gpt-5.5-pro, gpt-5.4-pro, gpt-5.4-mini, gpt-5.4-nano all have dash-separated
    // suffixes and would fuzzy-match to gpt-5.5 / gpt-5.4 without explicit entries,
    // producing wildly wrong costs (gpt-5.5-pro at $30/$180 vs gpt-5.5 at $5/$30).
    #[test]
    fn test_gpt5_family_no_fuzzy_bleed() {
        let db = PricingDatabase::with_defaults();
        for (model, exp_in, exp_out) in [
            ("gpt-5.5",          5.00_f64,  30.00_f64),
            ("gpt-5.5-pro",     30.00,     180.00),
            ("gpt-5.4",          2.50,      15.00),
            ("gpt-5.4-mini",     0.75,       4.50),
            ("gpt-5.4-nano",     0.20,       1.25),
            ("gpt-5.4-pro",     30.00,     180.00),
            ("gpt-5.3-codex",    1.75,      14.00),
            ("gpt-5.1-codex-mini", 0.25,    2.00),
        ] {
            let cost = db.calculate_cost(LLMProvider::OpenAI, model, 1_000_000, 1_000_000, None, None);
            assert!(
                (cost.input_cost - exp_in).abs() < 0.001,
                "{}: expected ${} input, got {}",
                model, exp_in, cost.input_cost
            );
            assert!(
                (cost.output_cost - exp_out).abs() < 0.001,
                "{}: expected ${} output, got {}",
                model, exp_out, cost.output_cost
            );
        }
    }

    // o1-pro and o3-pro would fuzzy-match to o1/o3 (both start with the prefix + "-")
    // and get dramatically wrong prices if not registered as explicit entries.
    #[test]
    fn test_o1_pro_and_o3_pro_not_misprice_as_o1_o3() {
        let db = PricingDatabase::with_defaults();
        for (model, exp_in, exp_out) in [
            ("o1-pro",          150.00_f64, 600.00_f64),
            ("o3-pro",           20.00,      80.00),
            ("codex-mini-latest", 1.50,       6.00),
        ] {
            let cost = db.calculate_cost(LLMProvider::OpenAI, model, 1_000_000, 1_000_000, None, None);
            assert!(
                (cost.input_cost - exp_in).abs() < 0.001,
                "{}: expected ${} input, got {}",
                model, exp_in, cost.input_cost
            );
            assert!(
                (cost.output_cost - exp_out).abs() < 0.001,
                "{}: expected ${} output, got {}",
                model, exp_out, cost.output_cost
            );
        }
    }

    // The May-2024 gpt-4o snapshot is $5/$15, not the current $2.50/$10.
    // It must be registered explicitly so it isn't fuzzy-matched to gpt-4o.
    #[test]
    fn test_gpt4o_old_snapshot_uses_higher_rate() {
        let db = PricingDatabase::with_defaults();
        let current = db.calculate_cost(LLMProvider::OpenAI, "gpt-4o", 1_000_000, 0, None, None);
        let old     = db.calculate_cost(LLMProvider::OpenAI, "gpt-4o-2024-05-13", 1_000_000, 0, None, None);
        assert!((current.input_cost - 2.50).abs() < 0.001, "gpt-4o should be $2.50");
        assert!((old.input_cost - 5.00).abs() < 0.001, "gpt-4o-2024-05-13 should be $5.00");
    }

    #[test]
    fn test_from_file_roundtrip() {
        // Minimal inline JSON — verifies PricingDatabase::from_file() produces
        // correct entries without touching the filesystem.
        let json = r#"{
            "schema_version": 1,
            "updated": "2026-01-01",
            "models": [
                { "provider": "anthropic", "model": "test-model",
                  "input_per_mtok": 7.00, "output_per_mtok": 21.00,
                  "cache_read_per_mtok": 0.70 }
            ]
        }"#;
        let file: PricingFile = serde_json::from_str(json).expect("should parse");
        let db = PricingDatabase::from_file(&file);
        let cost = db.calculate_cost(
            LLMProvider::Anthropic, "test-model", 1_000_000, 1_000_000, None, None,
        );
        assert!((cost.input_cost - 7.00).abs() < 0.001);
        assert!((cost.output_cost - 21.00).abs() < 0.001);
    }

    #[test]
    fn test_from_file_rejects_unknown_schema_version() {
        let json = r#"{"schema_version":99,"updated":"2026-01-01","models":[]}"#;
        let file: PricingFile = serde_json::from_str(json).expect("should parse");
        // from_file() should fall back to compiled-in defaults, not panic.
        let db = PricingDatabase::from_file(&file);
        // Compiled-in defaults always know gpt-4o.
        let cost = db.calculate_cost(LLMProvider::OpenAI, "gpt-4o", 1_000_000, 0, None, None);
        assert!((cost.input_cost - 2.50).abs() < 0.01);
    }

    #[test]
    fn test_repo_pricing_json_is_valid() {
        // The canonical pricing.json committed to the repo must parse cleanly
        // and produce a non-empty database that includes key models.
        let content = include_str!("../../pricing.json");
        let file: PricingFile = serde_json::from_str(content).expect("pricing.json should be valid JSON");
        assert_eq!(file.schema_version, 1, "schema_version must be 1");
        assert!(!file.models.is_empty(), "models list must not be empty");

        let db = PricingDatabase::from_file(&file);

        // Spot-check corrected models
        let opus = db.calculate_cost(LLMProvider::Anthropic, "claude-opus-4-7", 1_000_000, 0, None, None);
        assert!((opus.input_cost - 5.00).abs() < 0.01, "opus-4-7 input should be $5");

        let haiku = db.calculate_cost(LLMProvider::Anthropic, "claude-haiku-4-5", 0, 1_000_000, None, None);
        assert!((haiku.output_cost - 5.00).abs() < 0.01, "haiku-4-5 output should be $5");

        let gpt41 = db.calculate_cost(LLMProvider::OpenAI, "gpt-4.1", 1_000_000, 0, None, None);
        assert!((gpt41.input_cost - 2.00).abs() < 0.01, "gpt-4.1 input should be $2");
    }
}
