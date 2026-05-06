use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::parser::LLMProvider;

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

        db.add("openai", "gpt-4o", 2.50, 10.00, Some(1.25), None);
        db.add("openai", "gpt-4o-mini", 0.15, 0.60, Some(0.075), None);
        db.add("openai", "gpt-4-turbo", 10.00, 30.00, None, None);
        db.add("openai", "gpt-4", 30.00, 60.00, None, None);
        db.add("openai", "gpt-3.5-turbo", 0.50, 1.50, None, None);
        db.add("openai", "o1", 15.00, 60.00, Some(7.50), None);
        db.add("openai", "o1-mini", 3.00, 12.00, Some(1.50), None);
        db.add("openai", "o3-mini", 1.10, 4.40, Some(0.55), None);
        db.add("openai", "o4-mini", 1.10, 4.40, Some(0.275), None);

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
            "claude-sonnet-4-20250514",
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

        // Short-name aliases used by Claude Code and other clients
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
            "claude-opus-4-7",
            15.00,
            75.00,
            Some(1.50),
            Some(18.75),
        );
        db.add(
            "anthropic",
            "claude-haiku-4-5",
            0.80,
            4.00,
            Some(0.08),
            Some(1.00),
        );
        db.add(
            "anthropic",
            "claude-haiku-4-5-20251001",
            0.80,
            4.00,
            Some(0.08),
            Some(1.00),
        );

        db.add("google", "gemini-2.5-pro", 1.25, 10.00, None, None);
        db.add("google", "gemini-2.5-flash", 0.15, 0.60, None, None);
        db.add("google", "gemini-2.0-flash", 0.10, 0.40, None, None);
        db.add("google", "gemini-1.5-pro", 1.25, 5.00, None, None);
        db.add("google", "gemini-1.5-flash", 0.075, 0.30, None, None);

        // Cursor fallback — used when we can't detect the specific model.
        // Priced at Claude Sonnet rates (most common Cursor model tier).
        db.add("cursor", "cursor-unknown", 3.00, 15.00, None, None);

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
            // First check cursor-specific entries, then search other providers by model name.
            let cursor_key = format!("cursor:{}", model);
            self.models
                .get(&cursor_key)
                .or_else(|| self.cross_provider_match(model))
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
            .filter(|(_, v)| model.starts_with(&v.model) || v.model.starts_with(model))
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
    fn test_cursor_cross_provider_lookup() {
        let db = PricingDatabase::with_defaults();
        let cost = db.calculate_cost(
            LLMProvider::Cursor,
            "claude-sonnet-4-20250514",
            1_000_000,
            500_000,
            None,
            None,
        );
        // Should match Anthropic's claude-sonnet-4 pricing
        assert!((cost.input_cost - 3.00).abs() < 0.01);
        assert!((cost.output_cost - 7.50).abs() < 0.01);
        assert_eq!(cost.provider, "cursor");
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
}
