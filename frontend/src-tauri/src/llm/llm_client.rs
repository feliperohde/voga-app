//! One system + user prompt to the chat model chosen in Settings › AI: the built-in models
//! (crate::builtin), the Claude Code CLI (claude_code.rs), Anthropic's Messages API, or an
//! OpenAI-compatible chat/completions API (OpenAI, Groq, OpenRouter, Gemini, DeepSeek, Ollama,
//! a server the user set up).

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};

const TIMEOUT: Duration = Duration::from_secs(300);
const ANTHROPIC: &str = "https://api.anthropic.com/v1/messages";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAI,
    Claude,
    Groq,
    Ollama,
    OpenRouter,
    CustomOpenAI,
    ClaudeCode,
    /// llama.cpp on this computer
    BuiltInAI,
    Gemini,
    DeepSeek,
}

impl Provider {
    /// The name saved in Settings › AI.
    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name.to_ascii_lowercase().as_str() {
            "openai" => Self::OpenAI,
            "claude" => Self::Claude,
            "groq" => Self::Groq,
            "ollama" => Self::Ollama,
            "openrouter" => Self::OpenRouter,
            "custom-openai" => Self::CustomOpenAI,
            "claude-code" => Self::ClaudeCode,
            "builtin-ai" => Self::BuiltInAI,
            "gemini" => Self::Gemini,
            "deepseek" => Self::DeepSeek,
            _ => bail!("Unsupported AI provider: {name}"),
        })
    }

    /// Its id in crate::providers, for the hosted OpenAI-compatible APIs.
    fn hosted(self) -> Option<&'static str> {
        match self {
            Self::OpenAI => Some("openai"),
            Self::Groq => Some("groq"),
            Self::OpenRouter => Some("openrouter"),
            Self::Gemini => Some("gemini"),
            Self::DeepSeek => Some("deepseek"),
            _ => None,
        }
    }
}

/// Where a prompt goes.
pub struct Target<'a> {
    pub provider: Provider,
    pub model: &'a str,
    pub api_key: &'a str,
    /// Ollama's address, or the custom server's
    pub endpoint: Option<&'a str>,
}

/// The model's answer. `temperature` goes to every provider when set (0: repeatable analysis).
pub async fn generate(client: &Client, target: &Target<'_>, system: &str, user: &str, temperature: Option<f32>) -> Result<String> {
    match target.provider {
        Provider::BuiltInAI => crate::builtin::generate(target.model, system, user, temperature).await,
        Provider::ClaudeCode => super::claude_code::generate(target.model, system, user, None).await.map_err(|e| anyhow!(e)),
        Provider::Claude => {
            let mut body = json!({
                "model": target.model,
                // Answer and, on models that think by default, their thinking
                "max_tokens": 8192,
                "system": system,
                "messages": [{ "role": "user", "content": user }],
            });
            if let Some(temperature) = temperature {
                body["temperature"] = json!(temperature);
            }
            let request = client.post(ANTHROPIC).header("x-api-key", target.api_key).header("anthropic-version", "2023-06-01");
            let (status, text) = send(request, &body).await?;
            if !status.is_success() {
                bail!("Claude answered {status}: {text}");
            }
            anthropic_text(&serde_json::from_str(&text).context("Claude's answer")?)
        }
        _ => compatible(client, target, system, user, temperature).await,
    }
}

fn chat_url(target: &Target<'_>) -> Result<String> {
    if let Some(id) = target.provider.hosted() {
        let base = crate::providers::openai_base(id, None).context("no address for this provider")?;
        return Ok(format!("{base}/chat/completions"));
    }
    let endpoint = target.endpoint.map(|e| e.trim().trim_end_matches('/')).filter(|e| !e.is_empty());
    Ok(match target.provider {
        Provider::Ollama => format!("{}/v1/chat/completions", endpoint.unwrap_or(crate::settings::DEFAULT_OLLAMA)),
        _ => format!("{}/chat/completions", endpoint.context("The custom AI server's address is not set in Settings › AI")?),
    })
}

fn chat_body(provider: Provider, model: &str, system: &str, user: &str, temperature: Option<f32>) -> Value {
    let mut body = json!({
        "model": model,
        "messages": [{ "role": "system", "content": system }, { "role": "user", "content": user }],
    });
    if let Some(temperature) = temperature {
        body["temperature"] = json!(temperature);
    }
    // Ollama's thinking models answer straight away, as the others are asked to
    if provider == Provider::Ollama {
        body["reasoning_effort"] = json!("none");
    }
    body
}

async fn compatible(client: &Client, target: &Target<'_>, system: &str, user: &str, temperature: Option<f32>) -> Result<String> {
    let url = chat_url(target)?;
    let mut body = chat_body(target.provider, target.model, system, user, temperature);
    let request = || {
        let request = client.post(&url);
        if target.api_key.is_empty() { request } else { request.bearer_auth(target.api_key) }
    };
    let (mut status, mut text) = send(request(), &body).await?;
    // Ollama before 0.12 does not know reasoning_effort: once more without it
    if target.provider == Provider::Ollama && rejects_reasoning_effort(status, &text) {
        log::warn!("Ollama does not take reasoning_effort; asking again without it");
        body.as_object_mut().expect("an object").remove("reasoning_effort");
        (status, text) = send(request(), &body).await?;
    }
    if !status.is_success() {
        bail!("{url} answered {status}: {text}");
    }
    compatible_text(&serde_json::from_str(&text).with_context(|| format!("{url}'s answer"))?)
}

async fn send(request: reqwest::RequestBuilder, body: &Value) -> Result<(StatusCode, String)> {
    let response = request.json(body).timeout(TIMEOUT).send().await.map_err(|e| {
        if e.is_timeout() {
            anyhow!("the AI gave no answer in {} minutes", TIMEOUT.as_secs() / 60)
        } else {
            anyhow!("could not reach the AI: {e}")
        }
    })?;
    let status = response.status();
    Ok((status, response.text().await?))
}

/// The answer of a chat/completions reply (a thinking model's reasoning is left out).
fn compatible_text(reply: &Value) -> Result<String> {
    let message = reply["choices"].get(0).map(|choice| &choice["message"]).context("the AI's reply has no choices")?;
    Ok(message["content"].as_str().unwrap_or_default().trim().to_string())
}

/// The first text block of a Messages reply (thinking blocks are left out).
fn anthropic_text(reply: &Value) -> Result<String> {
    reply["content"]
        .as_array()
        .and_then(|blocks| blocks.iter().find(|block| block["type"] == "text"))
        .and_then(|block| block["text"].as_str())
        .map(|text| text.trim().to_string())
        .context("Claude's reply has no text")
}

/// A 400/422 about reasoning_effort, or about thinking set to "none".
fn rejects_reasoning_effort(status: StatusCode, body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    matches!(status.as_u16(), 400 | 422)
        && (body.contains("reasoning_effort")
            || (body.contains("think") && body.contains("none") && (body.contains("not supported") || body.contains("invalid"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn providers_parse_by_their_saved_names() {
        assert_eq!(Provider::parse("builtin-ai").unwrap(), Provider::BuiltInAI);
        assert_eq!(Provider::parse("Custom-OpenAI").unwrap(), Provider::CustomOpenAI);
        assert!(Provider::parse("bard").is_err());
    }

    #[test]
    fn only_ollama_is_asked_not_to_think_and_temperature_goes_to_all() {
        for provider in [Provider::OpenAI, Provider::Groq, Provider::OpenRouter, Provider::CustomOpenAI] {
            let body = chat_body(provider, "m", "s", "u", None);
            assert!(body.get("reasoning_effort").is_none() && body.get("temperature").is_none());
        }
        let body = chat_body(Provider::Ollama, "m", "s", "u", Some(0.0));
        assert_eq!(body["reasoning_effort"], "none");
        assert_eq!(body["temperature"], 0.0);
        assert_eq!(body["messages"][0]["role"], "system");
    }

    #[test]
    fn addresses_follow_the_provider() {
        let target = |provider, endpoint| Target { provider, model: "m", api_key: "", endpoint };
        assert_eq!(chat_url(&target(Provider::Ollama, None)).unwrap(), "http://localhost:11434/v1/chat/completions");
        assert_eq!(chat_url(&target(Provider::Ollama, Some("http://10.0.0.5:11434/"))).unwrap(), "http://10.0.0.5:11434/v1/chat/completions");
        assert_eq!(chat_url(&target(Provider::CustomOpenAI, Some("http://box/v1"))).unwrap(), "http://box/v1/chat/completions");
        assert!(chat_url(&target(Provider::CustomOpenAI, Some(" "))).is_err());
        assert!(chat_url(&target(Provider::Groq, None)).unwrap().ends_with("/chat/completions"));
    }

    #[test]
    fn replies_give_only_the_answer() {
        let reply = json!({"choices": [{"message": {"content": " An answer. ", "reasoning_content": "private"}}]});
        assert_eq!(compatible_text(&reply).unwrap(), "An answer.");
        assert_eq!(compatible_text(&json!({"choices": [{"message": {"content": null}}]})).unwrap(), "");
        assert!(compatible_text(&json!({"choices": []})).is_err());

        let claude = json!({"content": [{"type": "thinking", "thinking": "private"}, {"type": "text", "text": "An answer."}]});
        assert_eq!(anthropic_text(&claude).unwrap(), "An answer.");
        assert!(anthropic_text(&json!({"content": [{"type": "thinking"}]})).is_err());
    }

    #[test]
    fn old_ollama_rejections_are_recognized() {
        assert!(rejects_reasoning_effort(StatusCode::BAD_REQUEST, r#"{"error":{"param":"reasoning_effort"}}"#));
        assert!(rejects_reasoning_effort(StatusCode::UNPROCESSABLE_ENTITY, r#"{"message":"Unsupported parameter 'Reasoning_Effort'"}"#));
        assert!(rejects_reasoning_effort(StatusCode::BAD_REQUEST, r#"{"error":"think value \"none\" is not supported for this model"}"#));
        assert!(!rejects_reasoning_effort(StatusCode::BAD_REQUEST, r#"{"error":"invalid model"}"#));
        assert!(!rejects_reasoning_effort(StatusCode::UNAUTHORIZED, r#"{"error":{"param":"reasoning_effort"}}"#));
    }

    /// Answers each request with the next reply, handing back the bodies it received.
    async fn server(replies: Vec<(&'static str, &'static str)>) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let mut bodies = Vec::new();
            for (status, reply) in replies {
                // A connection opened and closed unused is skipped
                let (mut stream, request) = loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 4096];
                    let complete = loop {
                        let n = stream.read(&mut buffer).await.unwrap();
                        if n == 0 {
                            break false;
                        }
                        request.extend_from_slice(&buffer[..n]);
                        let text = String::from_utf8_lossy(&request).to_string();
                        if let Some(end) = text.find("\r\n\r\n") {
                            let length = text[..end]
                                .lines()
                                .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                                .unwrap_or(0);
                            if request.len() >= end + 4 + length {
                                break true;
                            }
                        }
                    };
                    if complete {
                        break (stream, request);
                    }
                };
                let text = String::from_utf8_lossy(&request).to_string();
                bodies.push(serde_json::from_str(&text[text.find("\r\n\r\n").unwrap() + 4..]).unwrap());
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            bodies
        });
        (address, handle)
    }

    #[tokio::test]
    async fn an_old_ollama_is_asked_again_without_reasoning_effort() {
        let (address, server) = server(vec![
            ("400 Bad Request", r#"{"error":"think value \"none\" is not supported for this model"}"#),
            ("200 OK", r#"{"choices":[{"message":{"content":"Uma resposta."}}]}"#),
        ])
        .await;
        let target = Target { provider: Provider::Ollama, model: "qwen3", api_key: "", endpoint: Some(&address) };
        let answer = generate(&Client::new(), &target, "sistema", "pergunta", Some(0.0)).await.unwrap();
        assert_eq!(answer, "Uma resposta.");
        let bodies = server.await.unwrap();
        assert_eq!(bodies[0]["reasoning_effort"], "none");
        assert!(bodies[1].get("reasoning_effort").is_none());
        assert_eq!(bodies[1]["temperature"], 0.0);
    }

    #[tokio::test]
    async fn other_errors_are_reported_with_the_servers_words() {
        let (address, server) = server(vec![("401 Unauthorized", r#"{"error":"bad key"}"#)]).await;
        let target = Target { provider: Provider::CustomOpenAI, model: "m", api_key: "k", endpoint: Some(&address) };
        let error = generate(&Client::new(), &target, "s", "u", None).await.unwrap_err().to_string();
        assert!(error.contains("401") && error.contains("bad key"), "{error}");
        server.await.unwrap();
    }
}
