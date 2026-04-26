use crate::settings::Settings;
use egui::Ui;

pub fn show(settings: &mut Settings, ui: &mut Ui) -> bool {
    let mut close = false;
    ui.vertical(|ui| {
        ui.heading("Settings");
        ui.separator();

        ui.checkbox(&mut settings.dark_mode, "Dark mode");
        ui.horizontal(|ui| {
            ui.label("Font size");
            ui.add(egui::Slider::new(&mut settings.font_size, 10.0..=24.0).step_by(1.0));
        });
        ui.checkbox(&mut settings.trim_whitespace_on_save, "Trim trailing whitespace on save");

        ui.horizontal(|ui| {
            ui.label("Server host");
            ui.text_edit_singleline(&mut settings.server_host)
                .on_hover_text("Defaults to cc.minecartchris.cc; SAMARI_DEV=1 overrides to localhost:8080");
        });

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("AI assistant (Ollama)").strong());
        ui.label(
            egui::RichText::new("Runs locally — start `ollama serve` and `ollama pull <model>` first.")
                .small()
                .weak(),
        );
        ui.horizontal(|ui| {
            ui.label("Ollama URL");
            ui.text_edit_singleline(&mut settings.ollama_url)
                .on_hover_text("e.g. http://localhost:11434");
        });
        ui.horizontal(|ui| {
            ui.label("Model");
            ui.text_edit_singleline(&mut settings.ollama_model)
                .on_hover_text("Tag of an installed Ollama model, e.g. qwen2.5-coder:7b");
        });

        ui.separator();
        if ui.button("Close").clicked() { close = true; }
    });
    close
}
