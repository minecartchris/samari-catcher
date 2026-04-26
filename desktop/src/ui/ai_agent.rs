//! Right-hand panel: AI assistant backed by a local Ollama daemon.
//!
//! The panel is per-session — each tab has its own running prompt / result —
//! and only enabled when there's an active file to edit. On Generate we hand
//! the active file's name + buffer + the user's instruction to the runtime
//! task; on completion the response shows up here, and the user explicitly
//! clicks Apply to overwrite the editor buffer.
//!
//! We never auto-apply; the user owns the change. We never run Ollama on a
//! remote endpoint — see [crate::ollama] for the http:// guard.

use egui::{Color32, RichText, ScrollArea, Ui};

use crate::session::{AiStatus, Session};
use crate::settings::Settings;

pub enum AiAction {
    Idle,
    /// User clicked Generate; payload is the trimmed prompt.
    Generate(String),
    /// Apply `last_result` to the active file's buffer.
    Apply,
    /// Drop `last_result` without applying.
    Discard,
    /// Clear an `Error` status so the user can try again.
    ResetError,
}

pub fn show(session: &mut Session, settings: &Settings, ui: &mut Ui) -> AiAction {
    let mut action = AiAction::Idle;

    ui.heading("AI assistant");
    ui.label(
        RichText::new(format!("{} @ {}", settings.ollama_model, settings.ollama_url))
            .small()
            .weak(),
    );

    let active_file = session.active_file.clone();
    if active_file.is_none() {
        ui.add_space(4.0);
        ui.label(
            RichText::new("Open a file in the editor to use the AI agent.")
                .italics()
                .weak(),
        );
        return action;
    }

    ui.separator();
    ui.label("Instruction:");
    ui.add(
        egui::TextEdit::multiline(&mut session.ai.prompt)
            .desired_rows(4)
            .hint_text("e.g. add a function that prints the current turtle fuel level"),
    );

    ui.add_space(4.0);
    let busy = matches!(session.ai.status, AiStatus::Running);
    let has_prompt = !session.ai.prompt.trim().is_empty();

    ui.horizontal(|ui| {
        let label = if busy { "Generating…" } else { "Generate" };
        let btn = ui.add_enabled(!busy && has_prompt, egui::Button::new(label));
        if btn.clicked() {
            action = AiAction::Generate(session.ai.prompt.trim().to_string());
        }
        if busy { ui.spinner(); }
    });

    match &session.ai.status {
        AiStatus::Idle | AiStatus::Running => {}
        AiStatus::Error(msg) => {
            ui.add_space(4.0);
            ui.colored_label(Color32::from_rgb(220, 90, 90), format!("Error: {msg}"));
            if ui.button("Dismiss").clicked() {
                action = AiAction::ResetError;
            }
        }
    }

    if let Some(mut result) = session.ai.last_result.clone() {
        ui.add_space(8.0);
        ui.separator();
        ui.label(RichText::new("Suggested replacement").strong());
        let preview_lines = result.lines().count();
        ui.label(
            RichText::new(format!("{preview_lines} lines, {} chars", result.len()))
                .small()
                .weak(),
        );
        ScrollArea::vertical()
            .max_height(220.0)
            .id_salt("ai_result_preview")
            .show(ui, |ui| {
                // `result` is a local clone; edits would be silently dropped,
                // but with `interactive(false)` the user can't make any.
                ui.add(
                    egui::TextEdit::multiline(&mut result)
                        .desired_width(f32::INFINITY)
                        .desired_rows(8)
                        .code_editor()
                        .interactive(false),
                );
            });
        ui.horizontal(|ui| {
            if ui.button("Apply to file").clicked() {
                action = AiAction::Apply;
            }
            if ui.button("Discard").clicked() {
                action = AiAction::Discard;
            }
        });
    }

    action
}
