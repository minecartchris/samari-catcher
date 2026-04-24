//! Terminal view for the desktop client.
//!
//! Uses the same rendering as the web interface with auto-sizing and keyboard input.

use egui::{FontFamily, FontId, Key, RichText, ScrollArea, Ui, Vec2};
use crate::session::Session;

const CELL_WIDTH: f32 = 6.0;
const CELL_HEIGHT: f32 = 9.0;
const MARGIN: f32 = 4.0;
const BOTTOM_PADDING: f32 = 40.0;

const BG_COLOR: egui::Color32 = egui::Color32::from_rgb(0, 0, 0);

pub fn show(session: &mut Session, ui: &mut Ui) {
    let status = match &session.status {
        crate::session::SessionStatus::Connecting => "Connecting...",
        crate::session::SessionStatus::Waiting => "Waiting for computer...",
        crate::session::SessionStatus::Connected => "Connected",
        crate::session::SessionStatus::LostConnection => "Lost connection",
        crate::session::SessionStatus::Errored(e) => {
            ui.label(format!("Error: {e}"));
            return;
        }
    };

    let info = if let Some(label) = &session.info.label {
        format!("{} (Computer #{})", label, session.info.computer_id.unwrap_or(0))
    } else if let Some(id) = session.info.computer_id {
        format!("Computer #{}", id)
    } else {
        "Computer".to_string()
    };

    ui.horizontal(|ui| {
        ui.label(RichText::new("Terminal").strong());
        ui.label(RichText::new(status).small().weak());
        if !info.is_empty() {
            ui.label(RichText::new(&info).small().weak());
        }
    });
    ui.separator();

    let t = &session.terminal;
    let (width, height) = (t.width as usize, t.height as usize);

    if width == 0 || height == 0 || t.text.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label("Waiting for terminal data...");
        });
        return;
    }

    let available = ui.available_size();
    let available_width = available[0];
    let available_height = available[1] - BOTTOM_PADDING;

    let base_width = (width as f32) * CELL_WIDTH + MARGIN * 2.0;
    let base_height = (height as f32) * CELL_HEIGHT + MARGIN * 2.0;

    let scale_x = if base_width > 0.0 {
        (available_width / base_width).floor().max(1.0)
    } else {
        1.0
    };
    let scale_y = if base_height > 0.0 {
        (available_height / base_height).floor().max(1.0)
    } else {
        1.0
    };
    let scale = scale_x.min(scale_y);

    let term_width = base_width * scale;
    let term_height = base_height * scale;

    let font = FontId::new(CELL_WIDTH * scale, FontFamily::Monospace);

    ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_viewport(ui, |ui, _viewport| {
            let pos = ui.cursor().min;
            let painter = ui.painter();

            let bg_rect = egui::Rect::from_min_size(pos, Vec2::new(term_width, term_height));
            painter.rect_filled(bg_rect, 0.0, BG_COLOR);

            for y in 0..height {
                let line_text = t.text.get(y).map(|s| s.as_str()).unwrap_or("");
                let line_fore = t.fore.get(y).map(|s| s.as_str()).unwrap_or("");
                let line_back = t.back.get(y).map(|s| s.as_str()).unwrap_or("");

                let line_y = pos.y + MARGIN * scale + (y as f32) * CELL_HEIGHT * scale;

                for x in 0..width {
                    let cell_x = pos.x + MARGIN * scale + (x as f32) * CELL_WIDTH * scale;

                    let bg = parse_cc_color(line_back, x);
                    let fg = parse_cc_color(line_fore, x);

                    let cell_rect = egui::Rect::from_min_size(
                        egui::pos2(cell_x, line_y),
                        Vec2::new(CELL_WIDTH * scale, CELL_HEIGHT * scale),
                    );
                    painter.rect_filled(cell_rect, 0.0, bg);

                    let ch = line_text.chars().nth(x).unwrap_or(' ');
                    if ch != ' ' {
                        let text_pos = egui::pos2(cell_x + 1.0, line_y);
                        painter.text(text_pos, egui::Align2::LEFT_TOP, ch, font.clone(), fg);
                    }
                }
            }

            let cx = t.cursor_x.min((width.saturating_sub(1)) as u32);
            let cy = t.cursor_y.min((height.saturating_sub(1)) as u32);
            let cursor_rect = egui::Rect::from_min_size(
                egui::pos2(
                    pos.x + MARGIN * scale + (cx as f32) * CELL_WIDTH * scale,
                    pos.y + MARGIN * scale + (cy as f32) * CELL_HEIGHT * scale
                ),
                Vec2::new(CELL_WIDTH * scale, CELL_HEIGHT * scale),
            );

            let cursor_fg = parse_cc_color(&t.cursor_fore, 0);
            let cursor_bg = parse_cc_color(&t.cursor_back, 0);

            painter.rect_filled(cursor_rect, 0.0, cursor_fg);

            if t.cursor_blink {
                painter.rect_filled(cursor_rect, 0.0, cursor_fg.linear_multiply(0.5));
            } else {
                let cursor_pos = egui::pos2(cursor_rect.min.x, cursor_rect.min.y);
                painter.text(cursor_pos, egui::Align2::LEFT_TOP, '_', font.clone(), cursor_bg);
            }
        });

    ui.input(|input| {
        for event in &input.events {
            match event {
                egui::Event::Key { key, pressed, .. } => {
                    if *pressed {
                        if let Some((cc_key, _args)) = map_key_to_cc(*key) {
                            session.send_key_event(cc_key, vec![false.into()]);
                        }
                    }
                }
                egui::Event::Text(text) => {
                    for ch in text.chars() {
                        if ch >= ' ' && ch != '\x7f' {
                            session.send_char(ch);
                        }
                    }
                }
                _ => {}
            }
        }
    });
}

fn parse_cc_color(color: &str, index: usize) -> egui::Color32 {
    if color.is_empty() {
        return egui::Color32::from_rgb(240, 240, 240);
    }

    let color_char = color.chars().nth(index.min(color.len().saturating_sub(1))).unwrap_or('f');

    match color_char.to_ascii_lowercase() {
        '0' => egui::Color32::from_rgb(240, 240, 240),
        '1' => egui::Color32::from_rgb(242, 178, 51),
        '2' => egui::Color32::from_rgb(229, 127, 216),
        '3' => egui::Color32::from_rgb(153, 178, 242),
        '4' => egui::Color32::from_rgb(222, 222, 108),
        '5' => egui::Color32::from_rgb(127, 204, 25),
        '6' => egui::Color32::from_rgb(242, 178, 204),
        '7' => egui::Color32::from_rgb(76, 76, 76),
        '8' => egui::Color32::from_rgb(153, 153, 153),
        '9' => egui::Color32::from_rgb(76, 153, 178),
        'a' => egui::Color32::from_rgb(178, 102, 229),
        'b' => egui::Color32::from_rgb(37, 49, 146),
        'c' => egui::Color32::from_rgb(127, 102, 76),
        'd' => egui::Color32::from_rgb(87, 166, 78),
        'e' => egui::Color32::from_rgb(204, 76, 76),
        'f' => egui::Color32::from_rgb(0, 0, 0),
        _ => egui::Color32::from_rgb(240, 240, 240),
    }
}

fn map_key_to_cc(key: Key) -> Option<(&'static str, Vec<serde_json::Value>)> {
    let cc_key = match key {
        Key::Enter => "enter",
        Key::Backspace => "backspace",
        Key::Tab => "tab",
        Key::Escape => "escape",
        Key::ArrowUp => "up", Key::ArrowDown => "down",
        Key::ArrowLeft => "left", Key::ArrowRight => "right",
        Key::Home => "home", Key::End => "end",
        Key::PageUp => "pageup", Key::PageDown => "pagedown",
        Key::Insert => "insert", Key::Delete => "delete",
        Key::F1 => "f1", Key::F2 => "f2", Key::F3 => "f3", Key::F4 => "f4",
        Key::F5 => "f5", Key::F6 => "f6", Key::F7 => "f7", Key::F8 => "f8",
        Key::F9 => "f9", Key::F10 => "f10", Key::F11 => "f11", Key::F12 => "f12",
        _ => return None,
    };
    Some((cc_key, vec![false.into()]))
}