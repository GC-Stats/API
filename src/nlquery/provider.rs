/*
    GC-Stats — API

    Resolves which LLM provider and API key a request should use: the
    server-held "platform" key, or a caller-supplied BYOK key for openai/
    anthropic. Pure/testable — no network calls here, that's llm::*.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use serde::{Deserialize, Serialize};

use crate::nlquery::config::NlQueryConfig;
use crate::nlquery::error::NlQueryError;

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderChoice {
    Platform,
    Openai,
    Anthropic,
}

pub enum ResolvedProvider {
    OpenAi { api_key: String, model: String },
    Anthropic { api_key: String, model: String },
}

impl std::fmt::Debug for ResolvedProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolvedProvider::OpenAi { model, .. } => {
                f.debug_struct("OpenAi").field("api_key", &"[redacted]").field("model", model).finish()
            }
            ResolvedProvider::Anthropic { model, .. } => {
                f.debug_struct("Anthropic").field("api_key", &"[redacted]").field("model", model).finish()
            }
        }
    }
}

pub fn resolve_provider(
    llm_provider: ProviderChoice,
    api_key: Option<&str>,
    config: &NlQueryConfig,
) -> Result<(ProviderChoice, ResolvedProvider), NlQueryError> {
    match llm_provider {
        ProviderChoice::Platform => {
            let resolved = match config.platform_llm_provider.as_str() {
                "anthropic" => ResolvedProvider::Anthropic {
                    api_key: config.platform_llm_api_key.clone(),
                    model: config.anthropic_model.clone(),
                },
                "openai" => ResolvedProvider::OpenAi {
                    api_key: config.platform_llm_api_key.clone(),
                    model: config.openai_model.clone(),
                },
                other => {
                    return Err(NlQueryError::Config(format!(
                        "unrecognized platform_llm_provider '{other}' (expected 'openai' or 'anthropic')"
                    )))
                }
            };
            Ok((ProviderChoice::Platform, resolved))
        }
        ProviderChoice::Openai => {
            let key = api_key
                .filter(|k| !k.is_empty())
                .ok_or_else(|| NlQueryError::LlmCallFailed("api_key is required for the openai provider".to_string()))?;
            Ok((
                ProviderChoice::Openai,
                ResolvedProvider::OpenAi { api_key: key.to_string(), model: config.openai_model.clone() },
            ))
        }
        ProviderChoice::Anthropic => {
            let key = api_key
                .filter(|k| !k.is_empty())
                .ok_or_else(|| NlQueryError::LlmCallFailed("api_key is required for the anthropic provider".to_string()))?;
            Ok((
                ProviderChoice::Anthropic,
                ResolvedProvider::Anthropic { api_key: key.to_string(), model: config.anthropic_model.clone() },
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_config(platform_provider: &str) -> NlQueryConfig {
        NlQueryConfig {
            cube_api_url: "http://cube:4000".to_string(),
            cube_api_secret: None,
            cube_schema_cache_ttl_secs: 3600,
            cube_views: vec!["kill_stats".to_string()],
            platform_llm_provider: platform_provider.to_string(),
            platform_llm_api_key: "platform-secret-key".to_string(),
            openai_model: "gpt-4o-mini".to_string(),
            anthropic_model: "claude-3-5-haiku".to_string(),
            timeout_ms: 12_000,
        }
    }

    #[test]
    fn platform_routes_to_configured_openai() {
        let config = mock_config("openai");
        let (used, resolved) = resolve_provider(ProviderChoice::Platform, None, &config).unwrap();
        assert_eq!(used, ProviderChoice::Platform);
        match resolved {
            ResolvedProvider::OpenAi { api_key, model } => {
                assert_eq!(api_key, "platform-secret-key");
                assert_eq!(model, "gpt-4o-mini");
            }
            _ => panic!("expected OpenAi"),
        }
    }

    #[test]
    fn platform_routes_to_configured_anthropic() {
        let config = mock_config("anthropic");
        let (used, resolved) = resolve_provider(ProviderChoice::Platform, None, &config).unwrap();
        assert_eq!(used, ProviderChoice::Platform);
        match resolved {
            ResolvedProvider::Anthropic { api_key, model } => {
                assert_eq!(api_key, "platform-secret-key");
                assert_eq!(model, "claude-3-5-haiku");
            }
            _ => panic!("expected Anthropic"),
        }
    }

    #[test]
    fn byok_openai_uses_caller_supplied_key() {
        let config = mock_config("openai");
        let (used, resolved) = resolve_provider(ProviderChoice::Openai, Some("sk-user-key"), &config).unwrap();
        assert_eq!(used, ProviderChoice::Openai);
        match resolved {
            ResolvedProvider::OpenAi { api_key, .. } => assert_eq!(api_key, "sk-user-key"),
            _ => panic!("expected OpenAi"),
        }
    }

    #[test]
    fn byok_anthropic_uses_caller_supplied_key() {
        let config = mock_config("openai");
        let (used, resolved) = resolve_provider(ProviderChoice::Anthropic, Some("sk-ant-user-key"), &config).unwrap();
        assert_eq!(used, ProviderChoice::Anthropic);
        match resolved {
            ResolvedProvider::Anthropic { api_key, .. } => assert_eq!(api_key, "sk-ant-user-key"),
            _ => panic!("expected Anthropic"),
        }
    }

    #[test]
    fn byok_without_api_key_is_rejected() {
        let config = mock_config("openai");
        let err = resolve_provider(ProviderChoice::Openai, None, &config).unwrap_err();
        assert!(matches!(err, NlQueryError::LlmCallFailed(_)));
    }

    #[test]
    fn byok_with_empty_api_key_is_rejected() {
        let config = mock_config("openai");
        let err = resolve_provider(ProviderChoice::Anthropic, Some(""), &config).unwrap_err();
        assert!(matches!(err, NlQueryError::LlmCallFailed(_)));
    }
}
