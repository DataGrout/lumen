use anyhow::{anyhow, Result};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::io::Read;
use tracing::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LLMProvider {
    OpenAI,
    Anthropic,
    Google,
    Cursor,
}

impl std::fmt::Display for LLMProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAI => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::Google => write!(f, "google"),
            Self::Cursor => write!(f, "cursor"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
    #[serde(default)]
    pub cache_creation_tokens: Option<u64>,
}

pub fn detect_provider(url: &str) -> Option<LLMProvider> {
    if url.contains("api.openai.com") || url.contains("openai.azure.com") {
        Some(LLMProvider::OpenAI)
    } else if url.contains("api.anthropic.com") || url.contains("claude.ai") {
        Some(LLMProvider::Anthropic)
    } else if url.contains("generativelanguage.googleapis.com") {
        Some(LLMProvider::Google)
    } else if url.contains(".cursor.sh") || url.contains(".cursorapi.com") {
        Some(LLMProvider::Cursor)
    } else {
        None
    }
}

pub fn extract_model(provider: LLMProvider, body: &str) -> Option<String> {
    match provider {
        LLMProvider::OpenAI | LLMProvider::Google => {
            let v: serde_json::Value = serde_json::from_str(body).ok()?;
            v["model"].as_str().map(|s| s.to_string())
        }
        LLMProvider::Anthropic => {
            // Top-level field first, then scan SSE lines (claude.ai may not have top-level model)
            serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v["model"].as_str().map(|s| s.to_string()))
                .or_else(|| extract_model_generic(body))
        }
        LLMProvider::Cursor => {
            // Only try JSON/SSE text parsing on the request body. Scanning raw bytes
            // on large binary protobuf payloads produces false positives ("o1" appears
            // in any random binary). Response bytes are scanned separately in the proxy
            // after the full response is received.
            extract_model_generic(body)
        }
    }
}

/// Known model prefixes/patterns to scan for in binary payloads.
const MODEL_PATTERNS: &[&str] = &[
    "claude-opus-4",
    "claude-sonnet-4",
    "claude-haiku-4",
    "claude-3.5-sonnet",
    "claude-3-5-sonnet",
    "claude-3.5-haiku",
    "claude-3-5-haiku",
    "claude-3-haiku",
    "claude-3-opus",
    "gpt-5.5",
    "gpt-5-mini",
    "gpt-5-nano",
    "gpt-5",
    "gpt-4o-mini",
    "gpt-4o",
    "gpt-4-turbo",
    "gpt-4",
    "gpt-3.5-turbo",
    "o4-mini",
    "o4",
    "o3-mini",
    "o3",
    "o1-mini",
    "o1",
    "gemini-2.5-pro",
    "gemini-2.5-flash",
    "gemini-2.0-flash",
    "gemini-1.5-pro",
    "gemini-1.5-flash",
    "cursor-small",
    "composer-2",
];

/// Try to decompress gzip bytes; returns None if not gzip or decompression fails.
fn try_gunzip(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 2 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return None;
    }
    let mut decoder = GzDecoder::new(bytes);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).ok()?;
    Some(out)
}

/// Decode a payload for model-name scanning:
/// - Plain gzip (BidiAppend requests start directly with gzip magic)
/// - gRPC framed responses: walk frames, decompress the first compressed one.
///   gRPC frame layout: [compression_flag: 1B][length: 4B BE][data: N B]
///   compression_flag=0x01 → data is gzip-compressed (gRPC spec).
///   RunSSE responses have a small uncompressed header frame first, then a compressed frame.
/// Returns (decoded_bytes, from_compressed) where from_compressed signals that short
/// patterns should be skipped (they appear naturally in decompressed conversation text).
fn decode_cursor_payload(bytes: &[u8]) -> Option<(Vec<u8>, bool)> {
    // Plain gzip (BidiAppend requests start with gzip magic directly)
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        return try_gunzip(bytes).map(|d| (d, true));
    }
    // Walk gRPC frames — responses may have an uncompressed header frame before the
    // compressed data frame. Stop at the first compressed frame we can successfully gunzip.
    let mut offset = 0;
    while offset + 5 <= bytes.len() {
        let compression_flag = bytes[offset];
        let length = u32::from_be_bytes([
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
        ]) as usize;
        if length > 50_000_000 || offset + 5 + length > bytes.len() {
            break;
        }
        if compression_flag == 0x01 {
            let frame_data = &bytes[offset + 5..offset + 5 + length];
            if let Some(decompressed) = try_gunzip(frame_data) {
                return Some((decompressed, true));
            }
        }
        if length == 0 {
            break; // guard against infinite loop on empty frames
        }
        offset += 5 + length;
    }
    None
}

/// Scan raw bytes for known model name strings (works for protobuf/gRPC payloads).
/// Handles plain gzip (BidiAppend) and gRPC-framed gzip (RunSSE/AgentService).
pub fn scan_bytes_for_model(bytes: &[u8]) -> Option<String> {
    let decoded = decode_cursor_payload(bytes);
    let (scan_bytes, from_compressed): (&[u8], bool) = match &decoded {
        Some((data, compressed)) => {
            // Cap at 8KB — model name is in the proto header, not the conversation body.
            let limit = data.len().min(8192);
            (&data[..limit], *compressed)
        }
        None => (bytes, false),
    };
    let text = String::from_utf8_lossy(scan_bytes);

    if from_compressed {
        let preview_len = {
            let max = text.len().min(300);
            let mut i = max;
            while i > 0 && !text.is_char_boundary(i) { i -= 1; }
            i
        };
        tracing::debug!(
            "CURSOR gRPC decoded {}B (capped {}B), preview: {}",
            decoded.as_ref().map(|(d, _)| d.len()).unwrap_or(0),
            scan_bytes.len(),
            &text[..preview_len]
        );
    }

    // Longest match first — MODEL_PATTERNS is ordered with longer/more specific first
    for pattern in MODEL_PATTERNS {
        // Short ambiguous patterns (≤4 chars: o1, o3, o4) appear in user prose inside
        // decompressed conversation context — skip them for compressed-source content.
        if from_compressed && pattern.len() <= 4 {
            continue;
        }
        if let Some(pos) = text.find(pattern) {
            // Short patterns require a non-model character before them to avoid matching
            // mid-word (e.g. "foo1" shouldn't match "o1").
            if pattern.len() <= 4 && pos > 0 {
                let prev = text.as_bytes()[pos - 1] as char;
                if prev.is_ascii_alphanumeric() || prev == '-' || prev == '.' || prev == '_' {
                    continue;
                }
            }
            // Extract the full model string: read until a non-model character
            let remaining = &text[pos..];
            let end = remaining
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '.' && c != '_')
                .unwrap_or(remaining.len());
            let model = &remaining[..end];
            if !model.is_empty() {
                if from_compressed {
                    info!("CURSOR gRPC model found: {}", model);
                }
                return Some(model.to_string());
            }
        }
    }

    // For decompressed gRPC content, fall back to system-prompt text patterns.
    // The system prompt may appear anywhere in the decompressed payload (after a long
    // conversation history, for example), so search the full decoded bytes directly
    // rather than a capped string slice — no allocation, O(n) byte scan.
    if from_compressed {
        if let Some((data, _)) = &decoded {
            let composer_marker = b"powered by Composer";
            if data
                .windows(composer_marker.len())
                .any(|w| w == composer_marker)
            {
                info!("CURSOR gRPC model from system prompt: composer");
                return Some("composer".to_string());
            }
            let codex_marker = b"You are Codex ";
            if let Some(pos) = data
                .windows(codex_marker.len())
                .position(|w| w == codex_marker)
            {
                let after = &data[pos + codex_marker.len()..];
                let ver_end = after
                    .iter()
                    .position(|&b| !b.is_ascii_alphanumeric() && b != b'.' && b != b'-')
                    .unwrap_or(after.len());
                if let Ok(ver) = std::str::from_utf8(&after[..ver_end.min(20)]) {
                    let ver = ver.trim_end_matches('.');
                    if !ver.is_empty() {
                        let model = format!("codex-{}", ver);
                        info!("CURSOR gRPC model from system prompt: {}", model);
                        return Some(model);
                    }
                }
            }
        }
        tracing::debug!("CURSOR gRPC no model pattern matched in decoded content");
    }
    None
}

/// Best-effort model extraction: scans for a "model" field at any depth.
pub fn extract_model_generic(body: &str) -> Option<String> {
    // Try top-level JSON first
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(m) = v["model"].as_str() {
            return Some(m.to_string());
        }
        if let Some(m) = v["modelId"].as_str() {
            return Some(m.to_string());
        }
    }
    // Scan SSE lines for model references (handles top-level and message_start nested format)
    for line in body.lines() {
        let json_str = line.strip_prefix("data: ").unwrap_or(line).trim();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
            if let Some(m) = v["model"].as_str() {
                return Some(m.to_string());
            }
            // Anthropic message_start: {"type":"message_start","message":{"model":"claude-...",...}}
            if let Some(m) = v["message"]["model"].as_str() {
                return Some(m.to_string());
            }
        }
    }
    None
}

pub fn extract_usage(provider: LLMProvider, body: &str) -> Result<TokenUsage> {
    match provider {
        LLMProvider::OpenAI => extract_openai_usage(body),
        LLMProvider::Anthropic => extract_anthropic_usage(body),
        LLMProvider::Google => extract_google_usage(body),
        LLMProvider::Cursor => try_extract_any_usage(body)
            .ok_or_else(|| anyhow!("No usage data found in Cursor response")),
    }
}

/// Estimate token counts from byte sizes (~4 bytes per token for English).
/// Applies a configurable overhead margin for system prompts.
const BYTES_PER_TOKEN: f64 = 4.0;
const SYSTEM_PROMPT_OVERHEAD_TOKENS: u64 = 800;

pub fn estimate_usage_from_bytes(request_bytes: u64, response_bytes: u64) -> TokenUsage {
    let input_estimate =
        (request_bytes as f64 / BYTES_PER_TOKEN) as u64 + SYSTEM_PROMPT_OVERHEAD_TOKENS;
    let output_estimate = (response_bytes as f64 / BYTES_PER_TOKEN) as u64;
    TokenUsage {
        input_tokens: input_estimate,
        output_tokens: output_estimate,
        total_tokens: input_estimate + output_estimate,
        cache_read_tokens: None,
        cache_creation_tokens: None,
    }
}

/// Scan a response body for any recognizable usage data — tries standard formats,
/// SSE lines, and nested JSON structures.
pub fn try_extract_any_usage(body: &str) -> Option<TokenUsage> {
    // Try standard formats directly
    if let Ok(u) = extract_openai_usage(body) {
        return Some(u);
    }
    if let Ok(u) = extract_anthropic_usage(body) {
        return Some(u);
    }
    if let Ok(u) = extract_google_usage(body) {
        return Some(u);
    }

    // Scan SSE lines (streaming responses) — last match wins (usage is typically at the end)
    let mut last_usage: Option<TokenUsage> = None;
    for line in body.lines() {
        let json_str = line.strip_prefix("data: ").unwrap_or(line).trim();
        if json_str.is_empty() || json_str == "[DONE]" {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
            if let Some(u) = try_usage_from_value(&v) {
                last_usage = Some(u);
            }
        }
    }
    last_usage
}

/// Try to pull token usage from a generic JSON value by looking for common field patterns.
fn try_usage_from_value(v: &serde_json::Value) -> Option<TokenUsage> {
    // OpenAI-style: usage.prompt_tokens / completion_tokens
    if let Some(u) = v.get("usage") {
        if let (Some(inp), Some(out)) = (
            u.get("prompt_tokens").and_then(|t| t.as_u64()),
            u.get("completion_tokens").and_then(|t| t.as_u64()),
        ) {
            let total = u
                .get("total_tokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(inp + out);
            let cache = u
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|t| t.as_u64());
            return Some(TokenUsage {
                input_tokens: inp,
                output_tokens: out,
                total_tokens: total,
                cache_read_tokens: cache,
                cache_creation_tokens: None,
            });
        }

        // Anthropic-style: usage.input_tokens / output_tokens
        if let (Some(inp), Some(out)) = (
            u.get("input_tokens").and_then(|t| t.as_u64()),
            u.get("output_tokens").and_then(|t| t.as_u64()),
        ) {
            let cache_read = u.get("cache_read_input_tokens").and_then(|t| t.as_u64());
            let cache_create = u
                .get("cache_creation_input_tokens")
                .and_then(|t| t.as_u64());
            return Some(TokenUsage {
                input_tokens: inp,
                output_tokens: out,
                total_tokens: inp + out,
                cache_read_tokens: cache_read,
                cache_creation_tokens: cache_create,
            });
        }
    }

    // Google-style: usageMetadata.promptTokenCount
    if let Some(u) = v.get("usageMetadata") {
        if let (Some(inp), Some(out)) = (
            u.get("promptTokenCount").and_then(|t| t.as_u64()),
            u.get("candidatesTokenCount").and_then(|t| t.as_u64()),
        ) {
            let total = u
                .get("totalTokenCount")
                .and_then(|t| t.as_u64())
                .unwrap_or(inp + out);
            return Some(TokenUsage {
                input_tokens: inp,
                output_tokens: out,
                total_tokens: total,
                cache_read_tokens: u.get("cachedContentTokenCount").and_then(|t| t.as_u64()),
                cache_creation_tokens: None,
            });
        }
    }

    None
}

#[derive(Deserialize)]
struct OpenAIResponse {
    #[serde(default)]
    usage: Option<OpenAIUsage>,
}

#[derive(Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    #[serde(default)]
    prompt_tokens_details: Option<OpenAIPromptDetails>,
}

#[derive(Deserialize)]
struct OpenAIPromptDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
}

fn extract_openai_usage(body: &str) -> Result<TokenUsage> {
    let resp: OpenAIResponse = serde_json::from_str(body)?;
    let usage = resp
        .usage
        .ok_or_else(|| anyhow!("No usage field in OpenAI response"))?;

    Ok(TokenUsage {
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        cache_read_tokens: usage.prompt_tokens_details.and_then(|d| d.cached_tokens),
        cache_creation_tokens: None,
    })
}

#[derive(Deserialize)]
struct AnthropicResponse {
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u64,
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
}

fn extract_anthropic_usage(body: &str) -> Result<TokenUsage> {
    if let Ok(resp) = serde_json::from_str::<AnthropicResponse>(body) {
        if let Some(usage) = resp.usage {
            let cr = usage.cache_read_input_tokens.unwrap_or(0);
            let cc = usage.cache_creation_input_tokens.unwrap_or(0);
            return Ok(TokenUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                total_tokens: usage.input_tokens + usage.output_tokens + cr + cc,
                cache_read_tokens: usage.cache_read_input_tokens,
                cache_creation_tokens: usage.cache_creation_input_tokens,
            });
        }
    }
    // Streaming SSE: tokens are split across message_start and message_delta events
    extract_anthropic_streaming_usage(body)
        .ok_or_else(|| anyhow!("No usage data in Anthropic response"))
}

/// Accumulate token counts from Anthropic streaming SSE.
///
/// Anthropic splits usage across two event types:
///   message_start  → message.usage.input_tokens  (+ cache fields)
///   message_delta  → usage.output_tokens
pub fn extract_anthropic_streaming_usage(body: &str) -> Option<TokenUsage> {
    let mut input_tokens: Option<u64> = None;
    let mut output_tokens: Option<u64> = None;
    let mut cache_read: Option<u64> = None;
    let mut cache_create: Option<u64> = None;

    for line in body.lines() {
        let json_str = line.strip_prefix("data: ").unwrap_or(line).trim();
        if json_str.is_empty() || json_str == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
            continue;
        };

        match v.get("type").and_then(|t| t.as_str()) {
            Some("message_start") => {
                if let Some(u) = v.pointer("/message/usage") {
                    if let Some(n) = u.get("input_tokens").and_then(|t| t.as_u64()) {
                        input_tokens = Some(n);
                    }
                    if let Some(n) = u.get("cache_read_input_tokens").and_then(|t| t.as_u64()) {
                        cache_read = Some(n);
                    }
                    if let Some(n) = u
                        .get("cache_creation_input_tokens")
                        .and_then(|t| t.as_u64())
                    {
                        cache_create = Some(n);
                    }
                }
            }
            Some("message_delta") => {
                if let Some(u) = v.get("usage") {
                    if let Some(n) = u.get("output_tokens").and_then(|t| t.as_u64()) {
                        output_tokens = Some(n);
                    }
                }
            }
            _ => {}
        }
    }

    let inp = input_tokens?;
    let out = output_tokens.unwrap_or(0);
    let cr = cache_read.unwrap_or(0);
    let cc = cache_create.unwrap_or(0);
    Some(TokenUsage {
        input_tokens: inp,
        output_tokens: out,
        total_tokens: inp + out + cr + cc,
        cache_read_tokens: cache_read,
        cache_creation_tokens: cache_create,
    })
}

#[derive(Deserialize)]
struct GoogleResponse {
    #[serde(rename = "usageMetadata", default)]
    usage_metadata: Option<GoogleUsageMetadata>,
}

#[derive(Deserialize)]
struct GoogleUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: u64,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: u64,
    #[serde(rename = "totalTokenCount")]
    total_token_count: u64,
    #[serde(rename = "cachedContentTokenCount", default)]
    cached_content_token_count: Option<u64>,
}

fn extract_google_usage(body: &str) -> Result<TokenUsage> {
    let resp: GoogleResponse = serde_json::from_str(body)?;
    let usage = resp
        .usage_metadata
        .ok_or_else(|| anyhow!("No usageMetadata in Google response"))?;

    Ok(TokenUsage {
        input_tokens: usage.prompt_token_count,
        output_tokens: usage.candidates_token_count,
        total_tokens: usage.total_token_count,
        cache_read_tokens: usage.cached_content_token_count,
        cache_creation_tokens: None,
    })
}

/// Extract usage from a single SSE line.
#[cfg_attr(not(test), allow(dead_code))]
pub fn extract_usage_from_sse_line(provider: LLMProvider, line: &str) -> Option<TokenUsage> {
    let json_str = line.strip_prefix("data: ")?.trim();
    if json_str == "[DONE]" {
        return None;
    }
    extract_usage(provider, json_str).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_provider() {
        assert_eq!(
            detect_provider("https://api.openai.com/v1/chat/completions"),
            Some(LLMProvider::OpenAI)
        );
        assert_eq!(
            detect_provider("https://api.anthropic.com/v1/messages"),
            Some(LLMProvider::Anthropic)
        );
        assert_eq!(detect_provider("https://example.com"), None);
    }

    #[test]
    fn test_detect_cursor_provider() {
        assert_eq!(
            detect_provider("https://api2.cursor.sh/v1/chat"),
            Some(LLMProvider::Cursor)
        );
        assert_eq!(
            detect_provider("https://us-east.api5.cursor.sh/agent"),
            Some(LLMProvider::Cursor)
        );
        assert_eq!(
            detect_provider("https://marketplace.cursorapi.com/ext"),
            Some(LLMProvider::Cursor)
        );
    }

    #[test]
    fn test_estimate_usage_from_bytes() {
        let usage = estimate_usage_from_bytes(4000, 2000);
        assert_eq!(usage.input_tokens, 1800); // 1000 + 800 overhead
        assert_eq!(usage.output_tokens, 500);
        assert_eq!(usage.total_tokens, 2300);
    }

    #[test]
    fn test_try_extract_any_usage_openai_style() {
        let body = r#"{"usage":{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150}}"#;
        let usage = try_extract_any_usage(body).unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
    }

    #[test]
    fn test_try_extract_any_usage_from_sse() {
        let body = "data: {\"text\": \"hello\"}\ndata: {\"text\": \"world\"}\ndata: {\"usage\":{\"input_tokens\":200,\"output_tokens\":80}}\ndata: [DONE]\n";
        let usage = try_extract_any_usage(body).unwrap();
        assert_eq!(usage.input_tokens, 200);
        assert_eq!(usage.output_tokens, 80);
    }

    #[test]
    fn test_try_extract_any_usage_none() {
        let body = r#"{"result": "ok", "no_usage_here": true}"#;
        assert!(try_extract_any_usage(body).is_none());
    }

    #[test]
    fn test_extract_model_generic_sse() {
        let body =
            "data: {\"model\": \"claude-sonnet-4-20250514\", \"text\": \"hi\"}\ndata: [DONE]\n";
        assert_eq!(
            extract_model_generic(body),
            Some("claude-sonnet-4-20250514".to_string())
        );
    }

    #[test]
    fn test_extract_openai_usage() {
        let body = r#"{"usage":{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150}}"#;
        let usage = extract_openai_usage(body).unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
    }

    #[test]
    fn test_extract_openai_cached() {
        let body = r#"{"usage":{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150,"prompt_tokens_details":{"cached_tokens":40}}}"#;
        let usage = extract_openai_usage(body).unwrap();
        assert_eq!(usage.cache_read_tokens, Some(40));
    }

    #[test]
    fn test_extract_anthropic_usage() {
        let body = r#"{"usage":{"input_tokens":80,"output_tokens":40,"cache_read_input_tokens":20,"cache_creation_input_tokens":10}}"#;
        let usage = extract_anthropic_usage(body).unwrap();
        assert_eq!(usage.input_tokens, 80);
        assert_eq!(usage.output_tokens, 40);
        assert_eq!(usage.total_tokens, 150); // 80 + 40 + 20 + 10
        assert_eq!(usage.cache_read_tokens, Some(20));
        assert_eq!(usage.cache_creation_tokens, Some(10));
    }

    #[test]
    fn test_extract_google_usage() {
        let body = r#"{"usageMetadata":{"promptTokenCount":60,"candidatesTokenCount":30,"totalTokenCount":90}}"#;
        let usage = extract_google_usage(body).unwrap();
        assert_eq!(usage.input_tokens, 60);
        assert_eq!(usage.output_tokens, 30);
        assert_eq!(usage.total_tokens, 90);
    }

    #[test]
    fn test_extract_sse_line() {
        let line =
            r#"data: {"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
        let usage = extract_usage_from_sse_line(LLMProvider::OpenAI, line).unwrap();
        assert_eq!(usage.total_tokens, 15);
    }

    #[test]
    fn test_sse_done() {
        let line = "data: [DONE]";
        assert!(extract_usage_from_sse_line(LLMProvider::OpenAI, line).is_none());
    }

    #[test]
    fn test_extract_anthropic_streaming_usage() {
        // Real Anthropic streaming SSE shape: input in message_start, output in message_delta
        let body = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":120,\"output_tokens\":0}}}\n",
            "data: {\"type\":\"content_block_start\",\"index\":0}\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":45}}\n",
            "data: {\"type\":\"message_stop\"}\n",
        );
        let usage = extract_anthropic_streaming_usage(body).unwrap();
        assert_eq!(usage.input_tokens, 120);
        assert_eq!(usage.output_tokens, 45);
        assert_eq!(usage.total_tokens, 165);
    }

    #[test]
    fn test_extract_anthropic_streaming_usage_with_cache() {
        let body = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":200,\"output_tokens\":0,\"cache_read_input_tokens\":150,\"cache_creation_input_tokens\":50}}}\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":30}}\n",
        );
        let usage = extract_anthropic_streaming_usage(body).unwrap();
        assert_eq!(usage.input_tokens, 200);
        assert_eq!(usage.output_tokens, 30);
        assert_eq!(usage.total_tokens, 430); // 200 + 30 + 150 + 50
        assert_eq!(usage.cache_read_tokens, Some(150));
        assert_eq!(usage.cache_creation_tokens, Some(50));
    }

    #[test]
    fn test_extract_anthropic_usage_falls_back_to_streaming() {
        // extract_anthropic_usage should handle SSE bodies via fallback
        let body = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":80,\"output_tokens\":0}}}\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":20}}\n",
        );
        let usage = extract_anthropic_usage(body).unwrap();
        assert_eq!(usage.input_tokens, 80);
        assert_eq!(usage.output_tokens, 20);
    }

    #[test]
    fn test_scan_bytes_for_model_claude() {
        // Simulate protobuf bytes with a model name embedded
        let mut bytes = vec![0x0A, 0x18]; // field tag + length prefix
        bytes.extend_from_slice(b"claude-sonnet-4-20250514");
        bytes.extend_from_slice(&[0x12, 0x05, b'h', b'e', b'l', b'l', b'o']);
        assert_eq!(
            scan_bytes_for_model(&bytes),
            Some("claude-sonnet-4-20250514".to_string())
        );
    }

    #[test]
    fn test_scan_bytes_for_model_gpt() {
        let bytes = b"\x00\x00some-prefix\x00gpt-4o-2024-08-06\x00more-stuff";
        assert_eq!(
            scan_bytes_for_model(bytes),
            Some("gpt-4o-2024-08-06".to_string())
        );
    }

    #[test]
    fn test_scan_bytes_for_model_none() {
        let bytes = b"no model names here, just regular bytes";
        assert!(scan_bytes_for_model(bytes).is_none());
    }

    #[test]
    fn test_scan_bytes_prefers_specific() {
        // "gpt-4o-mini" should match before "gpt-4o" since it's listed first
        let bytes = b"\x00gpt-4o-mini-2024\x00";
        assert_eq!(
            scan_bytes_for_model(bytes),
            Some("gpt-4o-mini-2024".to_string())
        );
    }
}
