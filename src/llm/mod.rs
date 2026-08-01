/*
    GC-Stats — API

    Declares the llm module: a small `LlmProvider` abstraction so the
    nl-query pipeline doesn't branch on provider throughout its business
    logic, plus one implementation per supported provider.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

pub mod anthropic;
pub mod openai;

use crate::nlquery::error::NlQueryError;
use crate::nlquery::prompt::SystemPrompt;
use crate::nlquery::query::CubeQuery;

/// Asks the underlying model to translate a user's question into a
/// `CubeQuery`, constrained via that provider's structured-output/tool-use
/// mechanism — never free text to parse.
pub trait LlmProvider {
    async fn generate_cube_query(
        &self,
        http: &reqwest::Client,
        system_prompt: &SystemPrompt,
        user_query: &str,
    ) -> Result<CubeQuery, NlQueryError>;
}
