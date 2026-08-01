/*
    GC-Stats — API

    OpenAI implementation of `LlmProvider`: forces a tool call against the
    Cube query JSON schema via the Chat Completions API so the response is
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

pub struct OpenAiProvider {
    pub api_key: String,
    pub model: String,
}

impl LlmProvider for OpenAiProvider {
    async fn generate_cube_query(
        &self,
        http: &reqwest::Client,
        system_prompt: &SystemPrompt,
        user_query: &str,
    ) -> Result<CubeQuery, NlQueryError> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system_prompt.flat() },
                { "role": "user", "content": user_query },
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": TOOL_NAME,
                    "description": "Build a Cube.dev query (measures, dimensions, filters) answering the user's question.",
                    "parameters": cube_query_schema(),
                }
            }],
            "tool_choice": { "type": "function", "function": { "name": TOOL_NAME } },
        });

        let resp = http
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| NlQueryError::LlmCallFailed(format!("OpenAI request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(NlQueryError::LlmCallFailed(format!("OpenAI returned {}", resp.status())));
        }

        let payload: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| NlQueryError::LlmCallFailed(format!("invalid OpenAI response: {e}")))?;

        let arguments = payload["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .ok_or_else(|| NlQueryError::LlmCallFailed("OpenAI response missing tool call arguments".to_string()))?;

        serde_json::from_str::<CubeQuery>(arguments)
            .map_err(|e| NlQueryError::InvalidCubeQuery(format!("could not parse Cube query from model output: {e}")))
    }
}
