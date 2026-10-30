//! "Claude Code (subscription)" provider: runs the user's own `claude` CLI in
//! headless mode, so requests use their Claude Code login instead of an API key.

use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

/// Desktop apps on macOS do not inherit the shell PATH, so look in the usual
/// install locations before falling back to a PATH lookup.
fn claude_binary() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    [
        home.join(".local/bin/claude"),
        home.join(".claude/local/claude"),
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .unwrap_or_else(|| PathBuf::from("claude"))
}

#[derive(serde::Deserialize)]
struct CliResult {
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    result: String,
}

fn parse_output(stdout: &[u8]) -> Result<String, String> {
    let parsed: CliResult = serde_json::from_slice(stdout)
        .map_err(|e| format!("Unexpected Claude Code output: {e}"))?;
    if parsed.is_error {
        return Err(format!("Claude Code returned an error: {}", parsed.result));
    }
    Ok(parsed.result.trim().to_string())
}

pub async fn generate(
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    cancellation_token: Option<&CancellationToken>,
) -> Result<String, String> {
    let mut command = tokio::process::Command::new(claude_binary());
    command
        .args(["-p", "--output-format", "json", "--model", model])
        .args(["--system-prompt", system_prompt])
        // Plain completion: no tools, MCP servers, settings or saved sessions
        .args(["--tools", "", "--strict-mcp-config", "--setting-sources", ""])
        .arg("--no-session-persistence")
        // An API key in the environment would take precedence over the subscription
        .env_remove("ANTHROPIC_API_KEY")
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command.spawn().map_err(|e| {
        format!("Could not start Claude Code ({e}). Install it and sign in with `claude` first.")
    })?;
    let mut stdin = child.stdin.take().ok_or("Claude Code stdin unavailable")?;
    stdin
        .write_all(user_prompt.as_bytes())
        .await
        .map_err(|e| format!("Failed to send prompt to Claude Code: {e}"))?;
    drop(stdin);

    let output = child.wait_with_output();
    let output = match cancellation_token {
        Some(token) => tokio::select! {
            _ = token.cancelled() => return Err("Summary generation was cancelled".to_string()),
            output = output => output,
        },
        None => output.await,
    }
    .map_err(|e| format!("Claude Code failed: {e}"))?;

    if output.stdout.is_empty() {
        return Err(format!(
            "Claude Code failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_output(&output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_success_and_error_results() {
        assert_eq!(parse_output(br#"{"is_error":false,"result":" hi \n"}"#), Ok("hi".into()));
        assert!(parse_output(br#"{"is_error":true,"result":"Not logged in"}"#)
            .unwrap_err()
            .contains("Not logged in"));
        assert!(parse_output(b"garbage").is_err());
    }
}
