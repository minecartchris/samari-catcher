//! Per-tab WebSocket session.
//!
//! One `Session` lives in `AppState` per open token. It owns a mpsc pair:
//!   - inbound  (tokio task → UI): `SessionEvent` items produced by the WS loop
//!   - outbound (UI → tokio task): encoded JSON strings to send as WS frames
//!
//! Keeping these as *synchronous* std mpsc for inbound (so `egui::App::update`
//! can drain them without an async context) and tokio::mpsc for outbound (so
//! the UI's `.send(...)` is still synchronous but the task can `.recv().await`)
//! is deliberate — see PLAN.md "egui + tokio".

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use crate::diff_patch::{compute_diff, Fragment};
use crate::protocol::{
    self, file_flags, FileActionEntry, FileConsumeEntry, FileConsumeResult, Packet,
    TerminalContents,
};
use crate::settings::Settings;
use crate::token::{fletcher32, Token};

#[derive(Debug)]
pub enum SessionEvent {
    Connected,
    Packet(Packet),
    Closed(String),
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    Connecting,
    /// Handshake complete but no `file:host` / `terminal:host` peer yet.
    Waiting,
    /// At least one host peer is present.
    Connected,
    LostConnection,
    Errored(String),
}

#[derive(Clone, Debug, Default)]
pub struct SessionInfo {
    pub computer_id: Option<i64>,
    pub label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TerminalState {
    pub width: u32,
    pub height: u32,
    pub text: Vec<String>,
    pub fore: Vec<String>,
    pub back: Vec<String>,
    pub cursor_x: u32,
    pub cursor_y: u32,
    pub cursor_blink: bool,
    pub cursor_fore: String,
    pub cursor_back: String,
    /// Palette from the host, keyed by single-char index ("0".."f"), value is
    /// a packed 0x00RRGGBB int. `None` until the first `TerminalContents`
    /// packet lands — renderers should fall back to `terminal_font::default_palette`.
    pub palette: Option<std::collections::HashMap<String, u32>>,
}

impl Default for TerminalState {
    fn default() -> Self {
        Self {
            width: 80,
            height: 25,
            text: vec![String::new(); 25],
            fore: vec![String::new(); 25],
            back: vec![String::new(); 25],
            cursor_x: 1,
            cursor_y: 1,
            cursor_blink: true,
            cursor_fore: "#ffffff".into(),
            cursor_back: "#000000".into(),
            palette: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileModel {
    pub name: String,
    pub read_only: bool,
    pub is_new: bool,
    /// Last contents we know the remote has.
    pub remote_contents: String,
    pub remote_checksum: u32,
    /// The text the user is editing. Starts equal to `remote_contents`.
    pub buffer: String,
    /// While a save is in flight, these hold what we sent. Cleared on
    /// `FileConsume::Ok`.
    pub pending_update: Option<PendingUpdate>,
}

#[derive(Clone, Debug)]
pub struct PendingUpdate {
    pub contents: String,
    pub checksum: u32,
}

impl FileModel {
    pub fn modified(&self) -> bool { self.buffer != self.remote_contents }
}

#[derive(Clone, Debug)]
pub struct Notification {
    pub id: String,
    pub kind: NotificationKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationKind { Ok, Warn, Error }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiStatus {
    Idle,
    Running,
    Error(String),
}

impl Default for AiStatus {
    fn default() -> Self { AiStatus::Idle }
}

#[derive(Default)]
pub struct AiState {
    /// Free-form instruction the user types into the agent panel.
    pub prompt: String,
    pub status: AiStatus,
    /// Most recent successful response, awaiting Apply / Discard.
    pub last_result: Option<String>,
    /// Channel the running task delivers its result on. Drained from `pump`.
    rx: Option<std_mpsc::Receiver<AiResult>>,
}

pub enum AiResult {
    Ok(String),
    Err(String),
}

pub struct Session {
    pub token: Token,
    pub status: SessionStatus,
    pub info: SessionInfo,
    pub files: Vec<FileModel>,
    pub active_file: Option<String>,
    pub notifications: Vec<Notification>,
    pub terminal: TerminalState,
    pub ai: AiState,

    inbound_rx: std_mpsc::Receiver<SessionEvent>,
    outbound_tx: tokio_mpsc::UnboundedSender<String>,
    _task: JoinHandle<()>,

    /// Held so the AI agent can spawn its own tokio tasks on the same runtime
    /// the WS task uses, and so the task can wake the UI on completion.
    runtime: Handle,
    egui_ctx: egui::Context,

    /// Set once we've seen a host peer in a ConnectionUpdate. Used to decide
    /// whether to render the "lost connection" vs "waiting for computer" view.
    pub has_connected: bool,
}

impl Session {
    pub fn spawn(
        token: Token,
        settings: &Settings,
        runtime: &Handle,
        egui_ctx: egui::Context,
    ) -> Self {
        let url = build_url(&token, &settings.server_host);

        let (in_tx, in_rx) = std_mpsc::channel();
        let (out_tx, out_rx) = tokio_mpsc::unbounded_channel();

        let task = runtime.spawn(run_ws(url, in_tx, out_rx, egui_ctx.clone()));

        Self {
            token,
            status: SessionStatus::Connecting,
            info: SessionInfo::default(),
            files: Vec::new(),
            active_file: None,
            notifications: Vec::new(),
            terminal: TerminalState::default(),
            ai: AiState::default(),
            inbound_rx: in_rx,
            outbound_tx: out_tx,
            _task: task,
            runtime: runtime.clone(),
            egui_ctx,
            has_connected: false,
        }
    }

    /// Apply any pending WS events to session state. Call once per egui frame
    /// for each session. Returns true if the session changed (caller may want
    /// to re-persist).
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        while let Ok(ev) = self.inbound_rx.try_recv() {
            changed = true;
            match ev {
                SessionEvent::Connected => {
                    self.status = SessionStatus::Waiting;
                }
                SessionEvent::Closed(reason) => {
                    self.status = if self.has_connected {
                        SessionStatus::LostConnection
                    } else {
                        SessionStatus::Errored(reason)
                    };
                }
                SessionEvent::Error(reason) => {
                    self.status = SessionStatus::Errored(reason);
                }
                SessionEvent::Packet(p) => self.handle_packet(p),
            }
        }

        // Drain the AI task channel — at most one completion per request.
        let mut clear_rx = false;
        if let Some(rx) = self.ai.rx.as_ref() {
            if let Ok(result) = rx.try_recv() {
                changed = true;
                clear_rx = true;
                match result {
                    AiResult::Ok(text) => {
                        self.ai.status = AiStatus::Idle;
                        self.ai.last_result = Some(text);
                    }
                    AiResult::Err(msg) => {
                        self.ai.status = AiStatus::Error(msg);
                    }
                }
            }
        }
        if clear_rx { self.ai.rx = None; }

        changed
    }

    /// Spawn a non-streaming Ollama request on the runtime. The result lands
    /// in `ai.last_result` (success) or `ai.status = Error(...)`. Pressing
    /// Generate while a request is already running is a no-op.
    pub fn start_ai_edit(
        &mut self,
        instruction: String,
        file_name: &str,
        file_contents: &str,
        settings: &Settings,
    ) {
        if matches!(self.ai.status, AiStatus::Running) { return; }

        let (tx, rx) = std_mpsc::channel();
        self.ai.rx = Some(rx);
        self.ai.status = AiStatus::Running;
        self.ai.last_result = None;

        let url = settings.ollama_url.clone();
        let model = settings.ollama_model.clone();
        let egui_ctx = self.egui_ctx.clone();

        let system = "You are a code editor assistant for ComputerCraft Lua \
            (and other languages where applicable). The user gives you the \
            current contents of a file plus an instruction. Reply with the \
            COMPLETE new contents of the file, with no commentary, no \
            explanations, no markdown code fences. Only the literal file \
            contents — anything else will be written into their file \
            verbatim.";

        let prompt = format!(
            "File path: {file_name}\n\n\
             === BEGIN current contents ===\n{file_contents}\n=== END current contents ===\n\n\
             Instruction:\n{instruction}\n\n\
             Reply with the new file contents only.",
        );

        self.runtime.spawn(async move {
            let result = crate::ollama::generate(
                crate::ollama::OllamaConfig { url: &url, model: &model },
                &prompt,
                Some(system),
            )
            .await;
            let msg = match result {
                Ok(text) => AiResult::Ok(text),
                Err(e) => AiResult::Err(format!("{e:#}")),
            };
            let _ = tx.send(msg);
            egui_ctx.request_repaint();
        });
    }

    fn handle_packet(&mut self, p: Packet) {
        match p {
            Packet::ConnectionPing => {
                // Echo right back — the server hangs up clients that don't.
                let _ = self.send(&Packet::ConnectionPing);
            }
            Packet::ConnectionUpdate { capabilities, .. } => {
                let has_host = capabilities.iter().any(|c| c == "terminal:host" || c == "file:host");
                if has_host {
                    self.status = SessionStatus::Connected;
                    self.has_connected = true;
                } else if self.has_connected {
                    self.status = SessionStatus::LostConnection;
                    self.info = SessionInfo::default();
                } else {
                    self.status = SessionStatus::Waiting;
                }
            }
            Packet::ConnectionAbuse { message } => {
                self.push_notification(NotificationKind::Warn, "abuse", message);
            }
            Packet::TerminalInfo { id, label } => {
                self.info = SessionInfo { computer_id: id, label };
            }
            Packet::TerminalContents(contents) => {
                self.apply_terminal_contents(contents);
            }
            Packet::FileAction { actions, .. } => self.apply_file_actions(actions),
            Packet::FileConsume { files, .. } => self.apply_file_consume(files),
            Packet::FileListing { .. } | Packet::FileRequest { .. } => {
                // We don't act as a file:host peer.
            }
            Packet::TerminalEvents { .. } => { /* outbound-only; ignore echoes */ }
        }
    }

    fn apply_file_actions(&mut self, actions: Vec<FileActionEntry>) {
        for a in actions {
            match a.action {
                0 => {
                    // Replace — either new file or force-update.
                    let read_only = a.flags & file_flags::READ_ONLY != 0;
                    let is_new = a.flags & file_flags::NEW != 0;
                    let open_flag = a.flags & file_flags::OPEN != 0;
                    let force = a.flags & file_flags::FORCE != 0;
                    let contents = a.contents.unwrap_or_default();
                    let checksum = fletcher32(&contents);

                    if let Some(file) = self.files.iter_mut().find(|f| f.name == a.file) {
                        if force || file.remote_checksum == a.checksum {
                            file.remote_contents = contents.clone();
                            file.remote_checksum = checksum;
                            file.buffer = contents;
                            file.is_new = false;
                            file.read_only = read_only;
                            self.remove_notification(&a.file, "update");
                        } else {
                            self.push_notification(
                                NotificationKind::Warn,
                                &format!("{}\0update", a.file),
                                format!("{} changed on the remote.", a.file),
                            );
                        }
                    } else {
                        let file = FileModel {
                            name: a.file.clone(),
                            read_only,
                            is_new,
                            remote_contents: contents.clone(),
                            remote_checksum: checksum,
                            buffer: contents,
                            pending_update: None,
                        };
                        self.files.push(file);
                    }
                    self.files.sort_by(|x, y| x.name.cmp(&y.name));
                    if open_flag { self.active_file = Some(a.file); }
                }
                2 => {
                    self.files.retain(|f| f.name != a.file);
                    if self.active_file.as_deref() == Some(a.file.as_str()) {
                        self.active_file = None;
                    }
                }
                _ => {
                    // action=1 (Patch) from server→client is possible per protocol
                    // but the CC host only sends Replace. If we start seeing
                    // Patch packets we'll need to add the applier; see PLAN.md.
                    log::warn!("file action {} not yet implemented", a.action);
                }
            }
        }
    }

    fn apply_file_consume(&mut self, files: Vec<FileConsumeEntry>) {
        for info in files {
            let Some(file) = self.files.iter_mut().find(|f| f.name == info.file) else { continue; };
            match info.result {
                FileConsumeResult::Ok => {
                    if let Some(pending) = file.pending_update.take() {
                        if pending.checksum == info.checksum {
                            file.remote_contents = pending.contents;
                            file.remote_checksum = pending.checksum;
                            self.remove_notification(&info.file, "update");
                        } else {
                            self.push_notification(
                                NotificationKind::Warn,
                                &format!("{}\0update", info.file),
                                format!("{} changed on the remote.", info.file),
                            );
                        }
                    }
                }
                FileConsumeResult::Reject => self.push_notification(
                    NotificationKind::Error,
                    &format!("{}\0update", info.file),
                    format!("{} couldn't be saved (remote was changed).", info.file),
                ),
                FileConsumeResult::Failure => self.push_notification(
                    NotificationKind::Error,
                    &format!("{}\0update", info.file),
                    format!("{} failed to save (read only?).", info.file),
                ),
            }
        }
    }

    pub fn save_active(&mut self, trim_whitespace: bool) -> Result<()> {
        let Some(name) = self.active_file.clone() else { return Ok(()) };
        self.save_file(&name, trim_whitespace)
    }

    pub fn save_file(&mut self, name: &str, trim_whitespace: bool) -> Result<()> {
        let Some(file) = self.files.iter_mut().find(|f| f.name == name) else { return Ok(()) };
        if file.read_only { return Ok(()) }

        let mut contents = file.buffer.clone();
        if trim_whitespace {
            contents = contents.lines()
                .map(|l| l.trim_end())
                .collect::<Vec<_>>()
                .join("\n");
            // Preserve trailing newline if the buffer had one.
            if file.buffer.ends_with('\n') && !contents.ends_with('\n') { contents.push('\n'); }
        }
        let new_checksum = fletcher32(&contents);

        let entry = if file.is_new {
            FileActionEntry {
                file: file.name.clone(),
                checksum: file.remote_checksum,
                flags: 0,
                action: 0, // Replace
                contents: Some(contents.clone()),
                delta: None,
            }
        } else {
            let delta = compute_diff(&file.remote_contents, &contents);
            FileActionEntry {
                file: file.name.clone(),
                checksum: file.remote_checksum,
                flags: 0,
                action: 1, // Patch
                contents: None,
                delta: Some(delta),
            }
        };

        file.pending_update = Some(PendingUpdate { contents, checksum: new_checksum });
        // Update buffer to the possibly-trimmed version so dirty state clears
        // once we get the Ok back.
        file.buffer = file.pending_update.as_ref().unwrap().contents.clone();
        let packet = Packet::FileAction { id: 0, actions: vec![entry] };
        self.send(&packet)
    }

    pub fn send(&self, packet: &Packet) -> Result<()> {
        let s = protocol::encode(packet)?;
        if s.len() > protocol::MAX_PACKET_SIZE {
            log::warn!("outbound packet {} bytes exceeds MAX_PACKET_SIZE ({}); server may drop it",
                s.len(), protocol::MAX_PACKET_SIZE);
        }
        self.outbound_tx.send(s).map_err(|_| anyhow::anyhow!("session task is gone"))?;
        Ok(())
    }

    /// Send a keyDown for `key_name` (CC key identifier such as `"enter"` or
     /// `"a"`). `repeat` is true for auto-repeat events.
    pub fn send_key_event(&self, key_name: &str, repeat: bool) {
        let packet = Packet::TerminalEvents {
            events: vec![protocol::TerminalEvent {
                name: "cloud_catcher_key".to_string(),
                args: vec![key_name.into(), repeat.into()],
            }],
        };
        if let Err(e) = self.send(&packet) {
            log::warn!("failed to send key event: {e}");
        }
    }

    pub fn send_key_up(&self, key_name: &str) {
        let packet = Packet::TerminalEvents {
            events: vec![protocol::TerminalEvent {
                name: "cloud_catcher_key_up".to_string(),
                args: vec![key_name.into()],
            }],
        };
        if let Err(e) = self.send(&packet) {
            log::warn!("failed to send key up: {e}");
        }
    }

    pub fn send_char(&self, ch: char) {
        let packet = Packet::TerminalEvents {
            events: vec![protocol::TerminalEvent {
                name: "char".to_string(),
                args: vec![ch.to_string().into()],
            }],
        };
        if let Err(e) = self.send(&packet) {
            log::warn!("failed to send char: {e}");
        }
    }

    pub fn push_notification(&mut self, kind: NotificationKind, id: &str, message: impl Into<String>) {
        self.notifications.retain(|n| n.id != id);
        self.notifications.push(Notification { id: id.into(), kind, message: message.into() });
    }

    pub fn remove_notification(&mut self, file: &str, category: &str) {
        let id = format!("{file}\0{category}");
        self.notifications.retain(|n| n.id != id);
    }

    fn apply_terminal_contents(&mut self, contents: TerminalContents) {
        let t = &mut self.terminal;
        t.width = contents.width.max(1).min(200);
        t.height = contents.height.max(1).min(100);

        let (w, h) = (t.width as usize, t.height as usize);

        // Initialize arrays with empty strings if needed
        while t.text.len() < h { t.text.push(String::new()); }
        while t.fore.len() < h { t.fore.push(String::new()); }
        while t.back.len() < h { t.back.push(String::new()); }

        // Truncate to current size
        t.text.truncate(h);
        t.fore.truncate(h);
        t.back.truncate(h);

        // Apply incoming data
        for y in 0..h {
            if y < contents.text.len() {
                let line = &contents.text[y];
                // Handle line width - CC uses 1-based indexing for cursor but text is just the content
                let line_width = line.chars().count().min(w * 2); // Allow some extra for safety
                let chars: Vec<char> = line.chars().take(line_width).collect();
                t.text[y] = chars.into_iter().collect();
            } else {
                t.text[y] = String::new();
            }

            if y < contents.fore.len() {
                t.fore[y] = contents.fore[y].clone();
            } else {
                t.fore[y] = String::new();
            }

            if y < contents.back.len() {
                t.back[y] = contents.back[y].clone();
            } else {
                t.back[y] = String::new();
            }
        }

        // Update cursor position (convert from 1-based to 0-based)
        t.cursor_x = contents.cursor_x.saturating_sub(1).min(t.width - 1);
        t.cursor_y = contents.cursor_y.saturating_sub(1).min(t.height - 1);
        t.cursor_blink = contents.cursor_blink;

        // Cursor colors
        t.cursor_fore = contents.cur_fore.unwrap_or_else(|| "#ffffff".into());
        t.cursor_back = contents.cur_back.unwrap_or_else(|| "#000000".into());

        if let Some(pal) = contents.palette {
            t.palette = Some(pal);
        }
    }

    #[allow(dead_code)]
    pub fn pending_patch_bytes(_delta: &[Fragment]) -> usize { 0 }
}

fn build_url(token: &Token, server_host: &str) -> String {
    if std::env::var("SAMARI_DEV").ok().as_deref() == Some("1") {
        format!("ws://localhost:8080/connect?id={}&capabilities=file:edit,terminal:view", token)
    } else {
        format!("wss://{}/connect?id={}&capabilities=file:edit,terminal:view", server_host, token)
    }
}

async fn run_ws(
    url: String,
    in_tx: std_mpsc::Sender<SessionEvent>,
    mut out_rx: tokio_mpsc::UnboundedReceiver<String>,
    egui_ctx: egui::Context,
) {
    let ws = match tokio_tungstenite::connect_async(&url).await {
        Ok((ws, _)) => ws,
        Err(e) => {
            let _ = in_tx.send(SessionEvent::Error(format!("connect: {e}")));
            egui_ctx.request_repaint();
            return;
        }
    };
    let _ = in_tx.send(SessionEvent::Connected);
    egui_ctx.request_repaint();

    let (mut sink, mut stream) = ws.split();
    let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Skip the first tick (fires immediately).
    ping_interval.tick().await;

    loop {
        tokio::select! {
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    match protocol::decode(&text) {
                        Ok(p) => {
                            let _ = in_tx.send(SessionEvent::Packet(p));
                            egui_ctx.request_repaint();
                        }
                        Err(e) => log::warn!("decode failed: {e}; frame: {text}"),
                    }
                }
                Some(Ok(Message::Binary(_))) => { /* server never sends binary */ }
                Some(Ok(Message::Ping(data))) => {
                    // tungstenite answers control pings for us, but being
                    // defensive doesn't hurt.
                    let _ = sink.send(Message::Pong(data)).await;
                }
                Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
                Some(Ok(Message::Close(frame))) => {
                    let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                    let _ = in_tx.send(SessionEvent::Closed(reason));
                    egui_ctx.request_repaint();
                    break;
                }
                Some(Err(e)) => {
                    let _ = in_tx.send(SessionEvent::Error(format!("stream: {e}")));
                    egui_ctx.request_repaint();
                    break;
                }
                None => {
                    let _ = in_tx.send(SessionEvent::Closed("stream ended".into()));
                    egui_ctx.request_repaint();
                    break;
                }
            },
            out = out_rx.recv() => match out {
                Some(s) => {
                    if let Err(e) = sink.send(Message::Text(s)).await {
                        let _ = in_tx.send(SessionEvent::Error(format!("send: {e}")));
                        egui_ctx.request_repaint();
                        break;
                    }
                }
                None => {
                    // UI side dropped — close cleanly.
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            },
            _ = ping_interval.tick() => {
                // Application-level ping (the server sends its own every 15s,
                // we echo those. This is just belt-and-braces for NAT keepalive).
                if let Ok(s) = protocol::encode(&Packet::ConnectionPing) {
                    let _ = sink.send(Message::Text(s)).await;
                }
            }
        }
    }
}
