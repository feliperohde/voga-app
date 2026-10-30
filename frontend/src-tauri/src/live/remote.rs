//! Live transcription on a provider's server instead of this computer (Assunta's remote
//! provider, trimmed): the OpenAI audio API (`POST <base>/audio/transcriptions`) of OpenAI,
//! Groq or a server of the user's, for computers that cannot run Whisper fast enough. The
//! session's audio leaves this computer, a stretch of speech at a time.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::time::Duration;

/// Shorter audio is not sent (Whisper servers reject it or invent text).
const MIN_SAMPLES: usize = super::SAMPLE_RATE as usize / 10;

pub struct RemoteTranscriber {
    client: reqwest::Client,
    base: String,
    key: String,
    model: String,
}

impl RemoteTranscriber {
    pub fn new(base: &str, key: &str, model: &str) -> Self {
        Self { client: crate::net::client_for(base), base: base.trim_end_matches('/').to_string(), key: key.to_string(), model: model.to_string() }
    }

    /// The text of a stretch of 16 kHz speech (Portuguese; `hint`: the names to spell).
    pub async fn transcribe(&self, samples: &[f32], hint: &str) -> Result<String> {
        if samples.len() < MIN_SAMPLES {
            return Ok(String::new());
        }
        let file = reqwest::multipart::Part::bytes(super::wav::encode(samples)).file_name("audio.wav").mime_str("audio/wav")?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", file)
            .text("model", self.model.clone())
            .text("language", "pt")
            .text("response_format", "json")
            .text("temperature", "0");
        if !hint.is_empty() {
            form = form.text("prompt", hint.to_string());
        }
        let mut request = self.client.post(format!("{}/audio/transcriptions", self.base)).timeout(Duration::from_secs(60)).multipart(form);
        if !self.key.is_empty() {
            request = request.bearer_auth(&self.key);
        }
        let response = request.send().await.with_context(|| format!("The transcription server at {} did not answer", self.base))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("The transcription server returned HTTP {status}: {}", body.chars().take(300).collect::<String>()));
        }
        #[derive(Deserialize)]
        struct Reply {
            text: String,
        }
        let reply: Reply = serde_json::from_str(&body).map_err(|_| anyhow!("Unexpected answer from the transcription server: {}", body.chars().take(200).collect::<String>()))?;
        Ok(super::whisper::clean(&reply.text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn speech_is_posted_as_multipart_audio_and_its_text_read() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = vec![0u8; 65536];
            // Until the multipart body is complete (its closing boundary)
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request);
                if text.contains("name=\"prompt\"") && text.trim_end().ends_with("--") {
                    break;
                }
            }
            let body = r#"{"text":" Eu votei contra a reforma. "}"#;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&request).to_string()
        });
        let remote = RemoteTranscriber::new(&format!("http://{addr}/openai/v1/"), "secret", "whisper-large-v3-turbo");
        assert_eq!(remote.transcribe(&vec![0.1; 16_000], "Marina Exemplo.").await.unwrap(), "Eu votei contra a reforma.");
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /openai/v1/audio/transcriptions "));
        assert!(request.to_lowercase().contains("authorization: bearer secret"));
        assert!(request.contains("name=\"model\"\r\n\r\nwhisper-large-v3-turbo"));
        assert!(request.contains("name=\"language\"\r\n\r\npt"));
        assert!(request.contains("name=\"prompt\"\r\n\r\nMarina Exemplo."));
        // Too short to send
        assert_eq!(remote.transcribe(&[0.1; 100], "").await.unwrap(), "");
    }
}
