//! Tiny async HTTP/1.1 client for a local Ollama daemon.
//!
//! Only does what we need: a single non-streaming POST to `/api/generate`.
//! Ollama is local-only and HTTP-only, so we skip TLS, chunked transfer,
//! compression, and connection re-use. `Connection: close` + `read_to_end`
//! is enough to bound the response.
//!
//! Adding `reqwest` would pull in hyper / h2 / a transitive TLS stack and
//! roughly double our binary size — overkill for one endpoint.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    stream: bool,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

pub struct OllamaConfig<'a> {
    pub url: &'a str,
    pub model: &'a str,
}

/// Issue a single non-streaming `/api/generate` call. Returns the assistant's
/// `response` field, with any leading / trailing markdown code-fence stripped.
pub async fn generate(
    cfg: OllamaConfig<'_>,
    prompt: &str,
    system: Option<&str>,
) -> Result<String> {
    let parsed = url::Url::parse(cfg.url).context("parsing ollama url")?;
    let scheme = parsed.scheme();
    if scheme != "http" {
        return Err(anyhow!(
            "only http:// is supported for ollama (got {scheme}://)"
        ));
    }
    let host = parsed.host_str().ok_or_else(|| anyhow!("no host in ollama url"))?;
    let port = parsed.port().unwrap_or(11434);

    let body = serde_json::to_vec(&GenerateRequest {
        model: cfg.model,
        prompt,
        system,
        stream: false,
    })?;

    let mut stream = TcpStream::connect((host, port))
        .await
        .with_context(|| format!("connect ollama at {host}:{port}"))?;
    let head = format!(
        "POST /api/generate HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         User-Agent: samari-catcher\r\n\
         Accept: application/json\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;

    let header_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("malformed http response (no header terminator)"))?;
    let head_str = std::str::from_utf8(&buf[..header_end]).unwrap_or("");
    let body_bytes = &buf[header_end + 4..];

    let status_line = head_str.lines().next().unwrap_or("");
    if !status_line.contains(" 200 ") {
        let snippet: String = std::str::from_utf8(body_bytes)
            .unwrap_or("<non-utf8>")
            .chars()
            .take(400)
            .collect();
        return Err(anyhow!("ollama HTTP error: {status_line} — {snippet}"));
    }

    let resp: GenerateResponse =
        serde_json::from_slice(body_bytes).with_context(|| {
            format!(
                "parsing ollama JSON: {}",
                std::str::from_utf8(body_bytes)
                    .unwrap_or("<non-utf8>")
                    .chars()
                    .take(200)
                    .collect::<String>()
            )
        })?;

    Ok(strip_code_fence(&resp.response))
}

/// Coding models often wrap their answer in a single ``` fence — strip one
/// matched pair so the editor can apply the result verbatim. Bare backticks
/// inside the file are left alone.
fn strip_code_fence(s: &str) -> String {
    let trimmed = s.trim_matches(|c: char| c == '\n' || c == '\r' || c == ' ' || c == '\t');
    let Some(after_open) = trimmed.strip_prefix("```") else {
        return s.to_string();
    };
    // Drop the language tag on the first line (e.g. ```lua\n).
    let inner = match after_open.split_once('\n') {
        Some((_lang, rest)) => rest,
        None => return s.to_string(),
    };
    match inner.rsplit_once("```") {
        Some((before_close, _trailing)) => before_close.trim_end_matches('\n').to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_fence_removes_lua_block() {
        let s = "```lua\nprint(1)\n```";
        assert_eq!(strip_code_fence(s), "print(1)");
    }

    #[test]
    fn strip_fence_no_fence_is_identity() {
        let s = "print(1)\n-- ``` not a fence";
        assert_eq!(strip_code_fence(s), s);
    }

    #[test]
    fn strip_fence_trims_outer_whitespace() {
        let s = "\n\n```python\nx = 1\n```\n\n";
        assert_eq!(strip_code_fence(s), "x = 1");
    }
}
