use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_dark_mode")]
    pub dark_mode: bool,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default = "default_trim_whitespace")]
    pub trim_whitespace_on_save: bool,
    /// Prod server host (no protocol). Overridden by `SAMARI_DEV=1` env → uses
    /// localhost:8080 with ws:// instead.
    #[serde(default = "default_server_host")]
    pub server_host: String,

    /// Base URL of the local Ollama HTTP server. Plain `http://` only.
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,
    /// Ollama model tag, e.g. `qwen2.5-coder:7b`, `deepseek-coder:6.7b`,
    /// `codellama:13b`. Whatever the user has `ollama pull`'d.
    #[serde(default = "default_ollama_model")]
    pub ollama_model: String,
}

fn default_dark_mode() -> bool { true }
fn default_font_size() -> f32 { 14.0 }
fn default_trim_whitespace() -> bool { true }
fn default_server_host() -> String { "cc.minecartchris.cc".into() }
fn default_ollama_url() -> String { "http://localhost:11434".into() }
fn default_ollama_model() -> String { "qwen2.5-coder:7b".into() }

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark_mode: default_dark_mode(),
            font_size: default_font_size(),
            trim_whitespace_on_save: default_trim_whitespace(),
            server_host: default_server_host(),
            ollama_url: default_ollama_url(),
            ollama_model: default_ollama_model(),
        }
    }
}
