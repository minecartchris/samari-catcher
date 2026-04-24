//! Lua syntax highlighting for the file editor.
//!
//! Uses `syntect` with its bundled default syntaxes + themes. We build a
//! `LayoutJob` per-call and let egui cache the resulting galley — syntect's
//! output for a whole file under ~2k lines is fast enough to run on every
//! frame without noticeable lag, so we don't bother caching per-file.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use once_cell::sync::Lazy;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

struct Engine {
    syntaxes: SyntaxSet,
    themes: ThemeSet,
}

static ENGINE: Lazy<Engine> = Lazy::new(|| Engine {
    syntaxes: SyntaxSet::load_defaults_nonewlines(),
    themes: ThemeSet::load_defaults(),
});

fn find_syntax(file_name: &str) -> &'static SyntaxReference {
    // Use the filename extension when available (most CC files are `.lua`);
    // fall back to the plain-text syntax so the editor still paints something.
    let ext = std::path::Path::new(file_name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("lua");
    ENGINE.syntaxes.find_syntax_by_extension(ext)
        .or_else(|| ENGINE.syntaxes.find_syntax_by_token(ext))
        .unwrap_or_else(|| ENGINE.syntaxes.find_syntax_plain_text())
}

fn theme_name(dark: bool) -> &'static str {
    if dark { "base16-ocean.dark" } else { "InspiredGitHub" }
}

/// Return a layouter closure suitable for `TextEdit::layouter(&mut ...)`.
/// The closure captures `file_name` + `dark` so egui can call it on demand
/// whenever the editor's text changes.
pub fn highlight(
    text: &str,
    file_name: &str,
    dark: bool,
    font: FontId,
    wrap_width: f32,
) -> LayoutJob {
    let syntax = find_syntax(file_name);
    let theme = &ENGINE.themes.themes[theme_name(dark)];
    let mut h = HighlightLines::new(syntax, theme);

    let mut job = LayoutJob {
        text: String::with_capacity(text.len()),
        ..Default::default()
    };
    job.wrap.max_width = wrap_width;

    for line in LinesWithEndings::from(text) {
        // If syntect stumbles on the input (e.g. an unclosed string runs to
        // EOF) we fall back to plain text for that line — don't crash the UI.
        let parts: Vec<(Style, &str)> = match h.highlight_line(line, &ENGINE.syntaxes) {
            Ok(r) => r,
            Err(_) => vec![(Style::default(), line)],
        };
        for (style, slice) in parts {
            let fmt = TextFormat {
                font_id: font.clone(),
                color: Color32::from_rgb(
                    style.foreground.r,
                    style.foreground.g,
                    style.foreground.b,
                ),
                ..Default::default()
            };
            job.append(slice, 0.0, fmt);
        }
    }

    job
}
