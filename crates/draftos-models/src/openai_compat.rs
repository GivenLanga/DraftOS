//! OpenAI-compatible chat-completions adapter. One implementation covers
//! OpenAI itself plus every endpoint speaking the same shape: Ollama, vLLM,
//! llama.cpp server, DeepSeek, Mistral, Qwen (CLAUDE.md §9).

use crate::{
    key, Capabilities, CompletionRequest, CompletionResponse, ModelAdapter, ModelError, Result,
    Role,
};
use serde_json::{json, Value};

pub struct OpenAiCompatAdapter {
    id: String,
    model: String,
    base_url: String,
    local: bool,
}

fn defaults_for(id: &str) -> (&'static str, &'static str, bool) {
    // (base_url, default model, local)
    match id {
        "openai" => ("https://api.openai.com/v1", "gpt-4o", false),
        "ollama" => ("http://localhost:11434/v1", "llama3.1", true),
        "vllm" => ("http://localhost:8000/v1", "", true),
        "deepseek" => ("https://api.deepseek.com/v1", "deepseek-chat", false),
        "mistral" => ("https://api.mistral.ai/v1", "mistral-large-latest", false),
        "qwen" => ("https://dashscope.aliyuncs.com/compatible-mode/v1", "qwen-plus", false),
        _ => ("http://localhost:8000/v1", "", true),
    }
}

impl OpenAiCompatAdapter {
    pub fn new(id: &str, model: Option<String>, base_url: Option<String>) -> Self {
        let (default_base, default_model, default_local) = defaults_for(id);
        let base_url = base_url.unwrap_or_else(|| default_base.to_string());
        // A cloud provider pointed at localhost is local in the way that
        // matters (no data leaves the machine), and vice versa.
        let local = default_local
            || base_url.contains("localhost")
            || base_url.contains("127.0.0.1");
        Self {
            id: id.to_string(),
            model: model.unwrap_or_else(|| default_model.to_string()),
            base_url,
            local,
        }
    }
}

pub(crate) fn build_body(model: &str, req: &CompletionRequest) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            json!({"role": role, "content": m.content})
        })
        .collect();
    let mut body = json!({
        "model": model,
        "max_tokens": req.max_tokens,
        "messages": messages,
    });
    if req.json {
        body["response_format"] = json!({"type": "json_object"});
    }
    body
}

pub(crate) fn parse_response(adapter: &str, v: &Value) -> Result<CompletionResponse> {
    let bad = |message: String| ModelError::BadResponse {
        adapter: adapter.to_string(),
        message,
    };
    let choice = v["choices"]
        .as_array()
        .and_then(|c| c.first())
        .ok_or_else(|| bad("missing choices".to_string()))?;
    let text = choice["message"]["content"]
        .as_str()
        .ok_or_else(|| bad("missing message content".to_string()))?
        .to_string();
    Ok(CompletionResponse {
        text,
        model: v["model"].as_str().unwrap_or_default().to_string(),
        input_tokens: v["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
        output_tokens: v["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        stop_reason: choice["finish_reason"].as_str().map(str::to_string),
    })
}

impl ModelAdapter for OpenAiCompatAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            json_mode: true,
            streaming: false,
            local: self.local,
            max_context: 128_000,
        }
    }

    fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        // Local endpoints (Ollama, vLLM, llama.cpp) usually need no key.
        let api_key = if self.local {
            key::resolve_key(&self.id)?
        } else {
            Some(key::require_key(&self.id)?)
        };
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = build_body(&self.model, req);
        tracing::debug!(adapter = %self.id, model = %self.model, "chat/completions request");
        let mut request = ureq::post(&url).set("content-type", "application/json");
        if let Some(k) = &api_key {
            request = request.set("authorization", &format!("Bearer {k}"));
        }
        match request.send_json(body) {
            Ok(resp) => {
                let v: Value = resp.into_json().map_err(|e| ModelError::BadResponse {
                    adapter: self.id.clone(),
                    message: e.to_string(),
                })?;
                parse_response(&self.id, &v)
            }
            Err(ureq::Error::Status(status, resp)) => Err(ModelError::Api {
                adapter: self.id.clone(),
                status,
                body: resp.into_string().unwrap_or_default(),
            }),
            Err(e) => Err(ModelError::Network {
                adapter: self.id.clone(),
                message: e.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatMessage;

    #[test]
    fn body_maps_roles_and_json_mode() {
        let req = CompletionRequest {
            messages: vec![ChatMessage::system("sys"), ChatMessage::user("hi")],
            max_tokens: 256,
            json: true,
        };
        let body = build_body("llama3.1", &req);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(body["response_format"]["type"], "json_object");
    }

    #[test]
    fn parses_choice_and_usage() {
        let v = serde_json::json!({
            "model": "llama3.1",
            "choices": [{"message": {"role": "assistant", "content": "hey"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 7, "completion_tokens": 3}
        });
        let out = parse_response("ollama", &v).unwrap();
        assert_eq!(out.text, "hey");
        assert_eq!(out.input_tokens, 7);
        assert_eq!(out.stop_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn ollama_defaults_are_local() {
        let a = OpenAiCompatAdapter::new("ollama", None, None);
        assert!(a.capabilities().local);
        let b = OpenAiCompatAdapter::new("openai", None, None);
        assert!(!b.capabilities().local);
    }

    /// End-to-end against a throwaway local HTTP server (no TLS, no network).
    #[test]
    fn complete_against_mock_server() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let body = r#"{"model":"test","choices":[{"message":{"role":"assistant","content":"mocked"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).unwrap();
        });
        let adapter =
            OpenAiCompatAdapter::new("ollama", Some("test".into()), Some(format!("http://{addr}")));
        let out = adapter
            .complete(&CompletionRequest {
                messages: vec![ChatMessage::user("hi")],
                max_tokens: 16,
                json: false,
            })
            .unwrap();
        assert_eq!(out.text, "mocked");
        handle.join().unwrap();
    }
}
