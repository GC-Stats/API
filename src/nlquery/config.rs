/*
    GC-Stats — API

    Configuration for the nl-query pipeline: Cube.dev connection details,
    the platform-held LLM provider/key used when the caller doesn't bring
    its own key, model names and timing budgets. Loaded once from the
    environment in `main()` and stored on `AppState`.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

#[derive(Clone)]
pub struct NlQueryConfig {
    pub cube_api_url: String,
    pub cube_api_secret: Option<String>,
    pub cube_schema_cache_ttl_secs: u64,
    pub cube_views: Vec<String>,
    pub platform_llm_provider: String,
    pub platform_llm_api_key: String,
    pub openai_model: String,
    pub anthropic_model: String,
    pub timeout_ms: u64,
}
