/// Shared logic for OpenAI-compatible (chat/completions) APIs.
///
/// Hosts [`OpenAiCompatProvider`] — a single provider parameterized by
/// `base_url` that covers every OpenAI-compatible vendor (OpenAI, OpenRouter,
/// NVIDIA, Hugging Face, Together, Groq, DeepSeek, …) — plus the request/response
/// helpers reused by the key-less providers (`GitHubCopilotProvider`, etc.).
use std::{future::Future, pin::Pin};

use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::{ContentBlock, LlmError, LlmProvider, LlmResponse, Message, ModelInfo, ModelLister};

/// Generic provider for any OpenAI-compatible chat/completions endpoint.
///
/// `base_url` is the API root without a trailing slash (e.g.
/// `https://openrouter.ai/api/v1`); requests hit `{base_url}/chat/completions`
/// and `{base_url}/models`.
pub struct OpenAiCompatProvider {
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAiCompatProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    async fn complete_inner(
        &self,
        messages: &[Message],
        tools: &[serde_json::Value],
    ) -> Result<LlmResponse, LlmError> {
        let body = build_body(&self.model, messages, tools);
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;
        let resp = check_status(resp).await?;
        let raw: RawResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;
        parse_response(raw)
    }

    async fn list_models_inner(&self) -> Result<Vec<ModelInfo>, LlmError> {
        let resp = self
            .client
            .get(format!("{}/models", self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        let resp = check_status(resp).await?;
        let raw: RawModels = resp
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;
        Ok(parse_models(raw))
    }
}

impl LlmProvider for OpenAiCompatProvider {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [serde_json::Value],
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>> {
        Box::pin(self.complete_inner(messages, tools))
    }
}

impl ModelLister for OpenAiCompatProvider {
    fn list_models(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ModelInfo>, LlmError>> + Send + '_>> {
        Box::pin(self.list_models_inner())
    }
}

/// Map a deserialized `/models` payload to canonical [`ModelInfo`] entries.
pub(crate) fn parse_models(raw: RawModels) -> Vec<ModelInfo> {
    raw.data
        .into_iter()
        .map(|m| ModelInfo {
            display_name: m.name.unwrap_or_else(|| m.id.clone()),
            id: m.id,
            context_window: m.context_length,
        })
        .collect()
}

/// Build the JSON body for a chat/completions request.
/// Converts Anthropic-style `input_schema` tool definitions to OpenAI `function` format.
pub(crate) fn build_body(
    model: &str,
    messages: &[Message],
    tools: &[serde_json::Value],
) -> serde_json::Value {
    let oai_tools: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name":        t["name"],
                    "description": t["description"],
                    "parameters":  t["input_schema"],
                }
            })
        })
        .collect();

    let mut body = json!({ "model": model, "messages": messages });
    if !oai_tools.is_empty() {
        body["tools"] = json!(oai_tools);
    }
    body
}

/// Consume a response and return an error for non-2xx status codes.
pub(crate) async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, LlmError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    if status == 401 {
        return Err(LlmError::AuthFailed);
    }
    if status == 429 {
        return Err(LlmError::RateLimited);
    }
    let text = resp.text().await.unwrap_or_default();
    Err(LlmError::InvalidResponse(format!("{status}: {text}")))
}

/// Parse a raw deserialized response into the canonical `LlmResponse`.
pub(crate) fn parse_response(raw: RawResponse) -> Result<LlmResponse, LlmError> {
    let choice = raw
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| LlmError::InvalidResponse("no choices in response".to_string()))?;

    let stop_reason = match choice.finish_reason.as_deref() {
        Some("tool_calls") => "tool_use".to_string(),
        Some("stop") | None => "end_turn".to_string(),
        Some(other) => other.to_string(),
    };

    let mut content: Vec<ContentBlock> = Vec::new();

    if let Some(text) = choice.message.content
        && !text.is_empty()
    {
        content.push(ContentBlock::Text { text });
    }

    if let Some(tool_calls) = choice.message.tool_calls {
        for tc in tool_calls {
            let input =
                serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Null);
            content.push(ContentBlock::ToolUse {
                id: tc.id,
                name: tc.function.name,
                input,
            });
        }
    }

    Ok(LlmResponse {
        content,
        stop_reason,
    })
}

// ── Deserialization types ─────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct RawResponse {
    pub(crate) choices: Vec<Choice>,
}

#[derive(Deserialize)]
pub(crate) struct Choice {
    pub(crate) finish_reason: Option<String>,
    pub(crate) message: RawMessage,
}

#[derive(Deserialize)]
pub(crate) struct RawMessage {
    pub(crate) content: Option<String>,
    pub(crate) tool_calls: Option<Vec<RawToolCall>>,
}

#[derive(Deserialize)]
pub(crate) struct RawToolCall {
    pub(crate) id: String,
    pub(crate) function: RawFunction,
}

#[derive(Deserialize)]
pub(crate) struct RawFunction {
    pub(crate) name: String,
    pub(crate) arguments: String,
}

/// `/models` response — `{ "data": [ { "id", "name"?, "context_length"? }, … ] }`.
/// Extra fields are ignored, so OpenAI / OpenRouter / NVIDIA / HF all deserialize.
#[derive(Deserialize)]
pub(crate) struct RawModels {
    pub(crate) data: Vec<RawModel>,
}

#[derive(Deserialize)]
pub(crate) struct RawModel {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) context_length: Option<u32>,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_body_includes_model_and_messages() {
        use crate::Message;
        let msgs = vec![Message::user("hello")];
        let body = build_body("gpt-4o", &msgs, &[]);
        assert_eq!(body["model"], "gpt-4o");
        assert!(body["messages"].is_array());
    }

    #[test]
    fn build_body_omits_tools_key_when_empty() {
        use crate::Message;
        let body = build_body("m", &[Message::user("x")], &[]);
        assert!(
            body.get("tools").is_none(),
            "tools should be absent when empty"
        );
    }

    #[test]
    fn build_body_converts_anthropic_tool_schema_to_openai_format() {
        use crate::Message;
        let schema = json!({
            "name": "read_file",
            "description": "Read a file",
            "input_schema": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] }
        });
        let body = build_body("m", &[Message::user("x")], &[schema]);
        let tool = &body["tools"][0];
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], "read_file");
        assert_eq!(tool["function"]["parameters"]["required"][0], "path");
    }

    #[test]
    fn parse_response_stop_maps_to_end_turn() {
        let raw = RawResponse {
            choices: vec![Choice {
                finish_reason: Some("stop".to_string()),
                message: RawMessage {
                    content: Some("hi".to_string()),
                    tool_calls: None,
                },
            }],
        };
        let r = parse_response(raw).expect("parse");
        assert_eq!(r.stop_reason, "end_turn");
    }

    #[test]
    fn parse_response_tool_calls_maps_to_tool_use() {
        let raw = RawResponse {
            choices: vec![Choice {
                finish_reason: Some("tool_calls".to_string()),
                message: RawMessage {
                    content: None,
                    tool_calls: Some(vec![RawToolCall {
                        id: "tc1".to_string(),
                        function: RawFunction {
                            name: "echo".to_string(),
                            arguments: r#"{"msg":"hi"}"#.to_string(),
                        },
                    }]),
                },
            }],
        };
        let r = parse_response(raw).expect("parse");
        assert_eq!(r.stop_reason, "tool_use");
        assert!(matches!(&r.content[0], ContentBlock::ToolUse { name, .. } if name == "echo"));
    }

    #[test]
    fn parse_response_none_finish_reason_maps_to_end_turn() {
        let raw = RawResponse {
            choices: vec![Choice {
                finish_reason: None,
                message: RawMessage {
                    content: Some("x".to_string()),
                    tool_calls: None,
                },
            }],
        };
        let r = parse_response(raw).expect("parse");
        assert_eq!(r.stop_reason, "end_turn");
    }

    #[test]
    fn parse_response_passthrough_unknown_finish_reason() {
        let raw = RawResponse {
            choices: vec![Choice {
                finish_reason: Some("max_tokens".to_string()),
                message: RawMessage {
                    content: None,
                    tool_calls: None,
                },
            }],
        };
        let r = parse_response(raw).expect("parse");
        assert_eq!(r.stop_reason, "max_tokens");
    }

    #[test]
    fn parse_response_empty_text_is_not_added_to_content() {
        let raw = RawResponse {
            choices: vec![Choice {
                finish_reason: Some("stop".to_string()),
                message: RawMessage {
                    content: Some(String::new()),
                    tool_calls: None,
                },
            }],
        };
        let r = parse_response(raw).expect("parse");
        assert!(
            r.content.is_empty(),
            "empty text should not produce a content block"
        );
    }

    #[test]
    fn parse_response_invalid_tool_args_fallback_to_null() {
        let raw = RawResponse {
            choices: vec![Choice {
                finish_reason: Some("tool_calls".to_string()),
                message: RawMessage {
                    content: None,
                    tool_calls: Some(vec![RawToolCall {
                        id: "x".to_string(),
                        function: RawFunction {
                            name: "t".to_string(),
                            arguments: "not valid json {{".to_string(),
                        },
                    }]),
                },
            }],
        };
        let r = parse_response(raw).expect("parse");
        match &r.content[0] {
            ContentBlock::ToolUse { input, .. } => assert!(input.is_null()),
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn parse_response_no_choices_is_error() {
        let raw = RawResponse { choices: vec![] };
        assert!(parse_response(raw).is_err());
    }

    #[test]
    fn provider_trims_trailing_slash_from_base_url() {
        let p = OpenAiCompatProvider::new("https://openrouter.ai/api/v1/", "k", "m");
        assert_eq!(p.base_url, "https://openrouter.ai/api/v1");
    }

    #[test]
    fn provider_is_both_llm_provider_and_model_lister() {
        use std::sync::Arc;
        let p = Arc::new(OpenAiCompatProvider::new("https://x/v1", "k", "gpt-4o"));
        let _llm: Arc<dyn LlmProvider> = p.clone();
        let _lister: Arc<dyn ModelLister> = p;
    }

    #[test]
    fn parse_models_maps_data_entries() {
        let raw: RawModels = serde_json::from_value(json!({
            "data": [
                { "id": "gpt-4o", "context_length": 128000 },
                { "id": "anthropic/claude", "name": "Claude" }
            ]
        }))
        .expect("deserialize");
        let models = parse_models(raw);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-4o");
        assert_eq!(models[0].display_name, "gpt-4o");
        assert_eq!(models[0].context_window, Some(128000));
        assert_eq!(models[1].display_name, "Claude");
        assert_eq!(models[1].context_window, None);
    }

    #[test]
    fn parse_models_ignores_unknown_fields() {
        let raw: RawModels = serde_json::from_value(json!({
            "object": "list",
            "data": [ { "id": "m", "owned_by": "x", "created": 123 } ]
        }))
        .expect("deserialize");
        let models = parse_models(raw);
        assert_eq!(models[0].id, "m");
    }
}
