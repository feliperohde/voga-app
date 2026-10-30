//! The configured chat model (Settings › AI): one place sets the provider, model and
//! credentials that every feature uses.

pub mod claude_code;
pub mod llm_client;

use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use sqlx::SqlitePool;

use crate::settings;
use llm_client::{generate, Provider, Target};

#[async_trait]
pub trait ChatModel: Send + Sync {
    async fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String>;
}

pub struct ConfiguredChatModel {
    client: Client,
    provider: Provider,
    model: String,
    api_key: String,
    /// Ollama's address, or the custom server's
    endpoint: Option<String>,
    temperature: Option<f32>,
}

impl ConfiguredChatModel {
    /// Temperature 0: the same text gives the same reading (claims, judgements).
    pub fn deterministic(mut self) -> Self {
        self.temperature = Some(0.0);
        self
    }

    pub async fn from_settings(pool: &SqlitePool) -> Result<Self> {
        let mut s = settings::load(pool).await?;
        let provider = Provider::parse(&s.provider)?;
        if provider == Provider::BuiltInAI {
            if s.model.trim().is_empty() {
                s.model = crate::builtin::downloaded_chat_model().unwrap_or_default();
            }
            if s.model.is_empty() || !crate::builtin::is_downloaded(&s.model) {
                bail!("The built-in model is not downloaded yet. Download it in Settings › AI.");
            }
        }
        if s.model.trim().is_empty() {
            bail!("No AI model configured. Choose one in Settings › AI.");
        }
        let api_key = settings::api_key(pool, &s.provider).await?.unwrap_or_default();
        let needs_key = !matches!(provider, Provider::Ollama | Provider::CustomOpenAI | Provider::ClaudeCode | Provider::BuiltInAI);
        if needs_key && api_key.is_empty() {
            bail!("No API key for {}. Add it in Settings › AI.", s.provider);
        }
        let endpoint = match provider {
            Provider::Ollama => s.ollama_endpoint.clone(),
            Provider::CustomOpenAI => s.custom_endpoint.clone(),
            _ => None,
        }
        .filter(|e| !e.trim().is_empty());
        let client = match provider {
            Provider::Ollama => crate::net::client_for(endpoint.as_deref().unwrap_or(settings::DEFAULT_OLLAMA)),
            Provider::CustomOpenAI => crate::net::client_for(endpoint.as_deref().unwrap_or_default()),
            _ => Client::new(),
        };
        Ok(Self { client, provider, model: s.model, api_key, endpoint, temperature: None })
    }
}

#[async_trait]
impl ChatModel for ConfiguredChatModel {
    async fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        let target = Target { provider: self.provider, model: &self.model, api_key: &self.api_key, endpoint: self.endpoint.as_deref() };
        generate(&self.client, &target, system_prompt, user_prompt, self.temperature).await
    }
}
