//! Terminal view for the desktop client.
//!
//! Renders using the same ComputerCraft bitmap-font atlas as the web client
//! (`node_modules/@squid-dev/cc-web-term/assets/term_font.png`). Glyphs are
//! blitted from the atlas and tinted by palette; keyboard input gets forwarded
//! as `cloud_catcher_key` / `cloud_catcher_key_up` / `char` terminal events.

use egui::{Color32, Key, Rect, RichText, Ui, Vec2};

use crate::session::{Session, SessionStatus};
use crate::terminal_font::{self, CELL_H, CELL_W, TERMINAL_MARGIN};

pub fn show(session: &mut Session, ui: &mut Ui) {
    let status = match &session.status {
        SessionStatus::Connecting => "Connecting…",
        SessionStatus::Waiting => "Waiting for computer…",
        SessionStatus::Connected => "Connected",
        SessionStatus::LostConnection => "Lost connection",
        SessionStatus::Errored(e) => {
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
        ui.label(RichText::new(&info).small().weak());
    });
    ui.separator();

    let (width, height) = (session.terminal.width as usize, session.terminal.height as usize);
    if width == 0 || height == 0 || session.terminal.text.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label("Waiting for terminal data…");
        });
        return;
    }

    // Pick the HD atlas when the integer scale would otherwise exceed 1 — it
    // looks noticeably crisper above 1x.
    let font = terminal_font::get(ui.ctx(), true);

    // Scale the whole grid up to fill the available area; stick to integer
    // multiples so the bitmap font stays pixel-aligned.
    let base_w = width as f32 * CELL_W + TERMINAL_MARGIN * 2.0;
    let base_h = height as f32 * CELL_H + TERMINAL_MARGIN * 2.0;
    let avail = ui.available_size_before_wrap();
    let scale_x = (avail.x / base_w).floor().max(1.0);
    let scale_y = (avail.y / base_h).floor().max(1.0);
    let scale = scale_x.min(scale_y);

    let term_w = base_w * scale;
    let term_h = base_h * scale;

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(term_w, term_h), egui::Sense::click());
    if resp.clicked() { resp.request_focus(); }

    let painter = ui.painter_at(rect);

    // Solid black backdrop first so any empty cells read as "off" regardless
    // of the underlying egui theme.
    painter.rect_filled(rect, 0.0, Color32::BLACK);

    let cell_w = CELL_W * scale;
    let cell_h = CELL_H * scale;
    let origin = egui::pos2(rect.min.x + TERMINAL_MARGIN * scale, rect.min.y + TERMINAL_MARGIN * scale);

    let palette = &session.terminal.palette;
    let resolve = |code: char| -> Color32 {
        if let Some(pal) = palette {
            if let Some(rgb) = pal.get(&code.to_string()) {
                return Color32::from_rgb(
                    ((rgb >> 16) & 0xFF) as u8,
                    ((rgb >> 8) & 0xFF) as u8,
                    (rgb & 0xFF) as u8,
                );
            }
        }
        terminal_font::default_palette(code)
    };

    for y in 0..height {
        let text = session.terminal.text.get(y).map(String::as_str).unwrap_or("");
        let fore = session.terminal.fore.get(y).map(String::as_str).unwrap_or("");
        let back = session.terminal.back.get(y).map(String::as_str).unwrap_or("");

        // Pre-collect chars so indexing by x is O(1) per cell rather than
        // re-walking the String's UTF-8 each time. The web protocol always
        // sends ASCII for a CC terminal so this is `width` bytes.
        let text_chars: Vec<char> = text.chars().collect();
        let fore_chars: Vec<char> = fore.chars().collect();
        let back_chars: Vec<char> = back.chars().collect();

        for x in 0..width {
            let cell_pos = egui::pos2(origin.x + x as f32 * cell_w, origin.y + y as f32 * cell_h);
            let cell_rect = Rect::from_min_size(cell_pos, Vec2::new(cell_w, cell_h));

            let bg_char = back_chars.get(x).copied().unwrap_or('f');
            painter.rect_filled(cell_rect, 0.0, resolve(bg_char));

            let ch = text_chars.get(x).copied().unwrap_or(' ');
            if ch != ' ' {
                let fg_char = fore_chars.get(x).copied().unwrap_or('0');
                let tint = resolve(fg_char);
                let uv = font.uv_for(ch as u32 as u8);
                painter.image(font.texture_id(), cell_rect, uv, tint);
            }
        }
    }

    // Cursor — drawn as an underscore tinted by cur_fore. Blink toggles at
    // ~2 Hz by default. The server sets `cursor_blink`; when false the cursor
    // stays on. We also ask egui to repaint so the blink actually animates.
    let cx = session.terminal.cursor_x as usize;
    let cy = session.terminal.cursor_y as usize;
    if cx < width && cy < height {
        let blink_on = if session.terminal.cursor_blink {
            ((ui.ctx().input(|i| i.time) * 2.0) as i64) % 2 == 0
        } else {
            true
        };
        if blink_on {
            let tint = parse_css_color(&session.terminal.cursor_fore)
                .unwrap_or_else(|| terminal_font::default_palette('0'));
            let cursor_pos = egui::pos2(origin.x + cx as f32 * cell_w, origin.y + cy as f32 * cell_h);
            let cursor_rect = Rect::from_min_size(cursor_pos, Vec2::new(cell_w, cell_h));
            let uv = font.uv_for(b'_');
            painter.image(font.texture_id(), cursor_rect, uv, tint);
        }
        if session.terminal.cursor_blink {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    // Keyboard input. Egui delivers both raw key events (useful for arrows,
    // enter, etc.) and a `Text` event carrying printable characters.
    ui.ctx().input(|input| {
        for event in &input.events {
            match event {
                egui::Event::Key { key, pressed, repeat, .. } => {
                    if let Some(cc_name) = map_key_to_cc(*key) {
                        if *pressed {
                            session.send_key_event(cc_name, *repeat);
                        } else {
                            session.send_key_up(cc_name);
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

fn parse_css_color(s: &str) -> Option<Color32> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    if hex.len() != 6 { return None; }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

fn map_key_to_cc(key: Key) -> Option<&'static str> {
    Some(match key {
        Key::Enter => "enter",
        Key::Backspace => "backspace",
        Key::Tab => "tab",
        Key::Escape => "escape",
        Key::Space => "space",
        Key::ArrowUp => "up",
        Key::ArrowDown => "down",
        Key::ArrowLeft => "left",
        Key::ArrowRight => "right",
        Key::Home => "home",
        Key::End => "end",
        Key::PageUp => "pageUp",
        Key::PageDown => "pageDown",
        Key::Insert => "insert",
        Key::Delete => "delete",
        Key::F1 => "f1", Key::F2 => "f2", Key::F3 => "f3", Key::F4 => "f4",
        Key::F5 => "f5", Key::F6 => "f6", Key::F7 => "f7", Key::F8 => "f8",
        Key::F9 => "f9", Key::F10 => "f10", Key::F11 => "f11", Key::F12 => "f12",
        Key::A => "a", Key::B => "b", Key::C => "c", Key::D => "d", Key::E => "e",
        Key::F => "f", Key::G => "g", Key::H => "h", Key::I => "i", Key::J => "j",
        Key::K => "k", Key::L => "l", Key::M => "m", Key::N => "n", Key::O => "o",
        Key::P => "p", Key::Q => "q", Key::R => "r", Key::S => "s", Key::T => "t",
        Key::U => "u", Key::V => "v", Key::W => "w", Key::X => "x", Key::Y => "y",
        Key::Z => "z",
        Key::Num0 => "zero",
        Key::Num1 => "one",
        Key::Num2 => "two",
        Key::Num3 => "three",
        Key::Num4 => "four",
        Key::Num5 => "five",
        Key::Num6 => "six",
        Key::Num7 => "seven",
        Key::Num8 => "eight",
        Key::Num9 => "nine",
        _ => return None,
    })
}
