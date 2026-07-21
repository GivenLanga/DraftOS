//! Anthropic (Claude) adapter — raw Messages API over HTTPS.
//! POST {base}/v1/messages with `x-api-key` + `anthropic-version` headers.

use crate::{
    key, Capabilities, CompletionRequest, CompletionResponse, ModelAdapter, ModelError, Result,
    Role,
};
use serde_json::{json, Value};

const DEFAULT_BASE: &str = "https://api.anthropic.com";
const DEFAULT_MODEL: &str = "claude-opus-4-8";
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicAdapter {
    model: String,
    base_url: String,
}

impl AnthropicAdapter {
    pub fn new(model: Option<String>, base_url: Option<String>) -> Self {
        Self {
            model: model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE.to_string()),
        }
    }
}

/// Build the /v1/messages request body. System messages are concatenated into
/// the top-level `system` field; when strict JSON is requested we say so in the
/// system prompt (the schema itself comes from the prompt compiler).
pub(crate) fn build_body(model: &str, req: &CompletionRequest) -> Value {
    let mut system_parts: Vec<&str> = Vec::new();
    let mut messages = Vec::new();
    for m in &req.messages {
        match m.role {
            Role::System => system_parts.push(&m.content),
            Role::User => messages.push(json!({"role": "user", "content": m.content})),
            Role::Assistant => messages.push(json!({"role": "assistant", "content": m.content})),
        }
    }
    if req.json {
        system_parts.push("Respond with a single valid JSON value and nothing else — no prose, no markdown fences.");
    }
    let mut body = json!({
        "model": model,
        "max_tokens": req.max_tokens,
        "messages": messages,
    });
    if !system_parts.is_empty() {
        body["system"] = json!(system_parts.join("\n\n"));
    }
    body
}

/// Pull the answer out of a /v1/messages response.
pub(crate) fn parse_response(v: &Value) -> Result<CompletionResponse> {
    let bad = |message: &str| ModelError::BadResponse {
        adapter: "anthropic".to_string(),
        message: message.to_string(),
    };
    let stop_reason = v["stop_reason"].as_str().map(str::to_string);
    if stop_reason.as_deref() == Some("refusal") {
        let why = v["stop_details"]["explanation"]
            .as_str()
            .unwrap_or("safety refusal")
            .to_string();
        return Err(ModelError::Refused(why));
    }
    let blocks = v["content"].as_array().ok_or_else(|| bad("missing content array"))?;
    let text: String = blocks
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect();
    if text.is_empty() {
        return Err(bad("no text blocks in response"));
    }
    Ok(CompletionResponse {
        text,
        model: v["model"].as_str().unwrap_or_default().to_string(),
        input_tokens: v["usage"]["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: v["usage"]["output_tokens"].as_u64().unwrap_or(0),
        stop_reason,
    })
}

impl ModelAdapter for AnthropicAdapter {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            json_mode: true,
            streaming: false,
            local: false,
            max_context: 1_000_000,
        }
    }

    fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let api_key = key::require_key("anthropic")?;
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let body = build_body(&self.model, req);
        tracing::debug!(model = %self.model, "anthropic request");
        let response = ureq::post(&url)
            .set("x-api-key", &api_key)
            .set("anthropic-version", API_VERSION)
            .set("content-type", "application/json")
            .send_json(body);
        match response {
            Ok(resp) => {
                let v: Value = resp.into_json().map_err(|e| ModelError::BadResponse {
                    adapter: "anthropic".to_string(),
                    message: e.to_string(),
                })?;
                parse_response(&v)
            }
            Err(ureq::Error::Status(status, resp)) => Err(ModelError::Api {
                adapter: "anthropic".to_string(),
                status,
                body: resp.into_string().unwrap_or_default(),
            }),
            Err(e) => Err(ModelError::Network {
                adapter: "anthropic".to_string(),
                message: e.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatMessage;

    fn req() -> CompletionRequest {
        CompletionRequest {
            messages: vec![
                ChatMessage::system("You are a legal drafting assistant."),
                ChatMessage::user("Adapt this clause."),
            ],
            max_tokens: 1024,
            json: true,
        }
    }

    #[test]
    fn body_extracts_system_and_requests_json() {
        let body = build_body("claude-opus-4-8", &req());
        assert_eq!(body["model"], "claude-opus-4-8");
        assert_eq!(body["max_tokens"], 1024);
        let system = body["system"].as_str().unwrap();
        assert!(system.starts_with("You are a legal drafting assistant."));
        assert!(system.contains("valid JSON"));
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn parses_text_and_usage() {
        let v = serde_json::json!({
            "model": "claude-opus-4-8",
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hello"}],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        let out = parse_response(&v).unwrap();
        assert_eq!(out.text, "hello");
        assert_eq!(out.input_tokens, 10);
        assert_eq!(out.output_tokens, 5);
    }

    #[test]
    fn refusal_is_an_error() {
        let v = serde_json::json!({
            "model": "claude-opus-4-8",
            "stop_reason": "refusal",
            "stop_details": {"explanation": "declined"},
            "content": [],
            "usage": {"input_tokens": 1, "output_tokens": 0}
        });
        assert!(matches!(parse_response(&v), Err(ModelError::Refused(_))));
    }
}
