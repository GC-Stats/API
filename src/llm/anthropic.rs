/*
    GC-Stats — API

    Anthropic implementation of `LlmProvider`: forces tool use against the
    Cube query JSON schema via the Messages API so the response is
    structured output, not free text to parse.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use crate::llm::LlmProvider;
use crate::nlquery::error::NlQueryError;
use crate::nlquery::prompt::SystemPrompt;
use crate::nlquery::query::{cube_query_schema, CubeQuery};

const TOOL_NAME: &str = "build_cube_query";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    pub api_key: String,
    pub model: String,
}

impl LlmProvider for AnthropicProvider {
    async fn generate_cube_query(
        &self,
        http: &reqwest::Client,
        system_prompt: &SystemPrompt,
        user_query: &str,
    ) -> Result<CubeQuery, NlQueryError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 1024,
            "system": [
                { "type": "text", "text": system_prompt.instructions },
                {
                    "type": "text",
                    "text": system_prompt.catalog,
                    "cache_control": { "type": "ephemeral" },
                },
            ],
            "messages": [{ "role": "user", "content": user_query }],
            "tools": [{
                "name": TOOL_NAME,
                "description": "Build a Cube.dev query (measures, dimensions, filters) answering the user's question.",
                "input_schema": cube_query_schema(),
            }],
            "tool_choice": { "type": "tool", "name": TOOL_NAME },
        });

        let resp = http
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|e| NlQueryError::LlmCallFailed(format!("Anthropic request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(NlQueryError::LlmCallFailed(format!("Anthropic returned {}", resp.status())));
        }

        let payload: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| NlQueryError::LlmCallFailed(format!("invalid Anthropic response: {e}")))?;

        tracing::debug!(
            cache_creation_input_tokens = payload["usage"]["cache_creation_input_tokens"].as_i64().unwrap_or(0),
            cache_read_input_tokens = payload["usage"]["cache_read_input_tokens"].as_i64().unwrap_or(0),
            input_tokens = payload["usage"]["input_tokens"].as_i64().unwrap_or(0),
            "Anthropic prompt cache usage"
        );

        let content = payload["content"]
            .as_array()
            .ok_or_else(|| NlQueryError::LlmCallFailed("Anthropic response missing content".to_string()))?;

        let tool_input = content
            .iter()
            .find(|block| block["type"] == "tool_use")
            .and_then(|block| block.get("input"))
            .ok_or_else(|| NlQueryError::LlmCallFailed("Anthropic response missing tool_use block".to_string()))?;

        serde_json::from_value::<CubeQuery>(tool_input.clone())
            .map_err(|e| NlQueryError::InvalidCubeQuery(format!("could not parse Cube query from model output: {e}")))
    }
}
