# Samurai Catcher — Rust Desktop App

Native Rust companion to the web interface at `cc.minecartchris.cc`. Same WebSocket protocol, same tokens. **Focus is file editing**; the terminal view is a stub (button to open in browser) until the editor is solid.

This doc is the source of truth for scope, architecture, and progress — designed so another AI/human can continue mid-stream without re-deriving decisions.

---

## Scope

### In scope (v1)
- Multi-tab sessions (one WebSocket per saved computer token).
- Tab bar UI: add / close / select / `+token` (prompt to attach an existing token) / import / export.
- Persistent token list + active tab + settings (config dir via `directories` crate).
- Import / export saved computers as JSON (native file dialogs via `rfd`).
- File list sidebar per session (receives `FileAction` packets from the CC client).
- File editor with Lua monospace text editing, save (Ctrl+S) → sends `FileAction.Patch` (or `Replace` for new files), handles `FileConsume` results.
- Settings dialog (dark/light, font size, trim whitespace on save, show invisible).

### Out of scope for v1 (stubbed or deferred)
- **Terminal rendering** — for now the "terminal" tab shows a placeholder with an "Open in browser" button → `https://cc.minecartchris.cc/?id=<token>`. Do this properly once the file editor is stable.
- Lua syntax highlighting. Plain monospace text is enough for v1. `syntect` can be added later.
- In-app completion / LSP. Not needed.
- Palette/theme editor. Use egui's built-in dark/light.
- Auto-update, code signing, installer. Just `cargo run --release` gives a working binary.

---

## Architecture

### Threading model
- UI thread: runs `eframe` main loop, owns `AppState`.
- Per-session tokio task: opens the WebSocket for one token, parses incoming packets, forwards them to the UI thread via `std::sync::mpsc::Sender`. Reads outgoing packets from an `mpsc::UnboundedReceiver<String>` written by the UI thread.
- `egui::Context::request_repaint()` is called from the session task when a packet arrives, so the UI wakes up and drains the inbound channel.

### Module layout (`desktop/src/`)
```
main.rs        entry: env_logger + eframe::run_native
app.rs         AppState + eframe::App impl; top-level update() drains channels, dispatches to UI submodules
token.rs       Token type (32-char alnum), gen/check, fletcher32 checksum
protocol.rs    Packet enum + PacketCode (numeric enum via serde_repr) + FileAction / FileActionFlags / FileConsume / Fragment
diff_patch.rs  char-level diff (similar::TextDiff::from_chars) → Vec<Fragment>; applier for Patch
session.rs     Session struct (held by AppState, one per tab): terminal state stub, file list, editor buffers, inbound_rx, outbound_tx, task JoinHandle. spawn_session() starts the WS task.
storage.rs     Paths via directories::ProjectDirs("cc","minecartchris","samari-catcher"). load/save Tabs{tokens, active}, Settings. Import/export JSON.
settings.rs    Settings struct + settings dialog egui UI.
ui/
  mod.rs       re-exports
  tabs.rs      tab bar: add, close, select, +token, import, export buttons
  files.rs     file list + editor UI (TextEdit, save hotkey, dirty markers)
  terminal.rs  placeholder UI with "Open in browser" button
```

### Crate choices (see `Cargo.toml`)
- `eframe` + `egui` — UI.
- `tokio` + `tokio-tungstenite` + `futures-util` — async WebSocket client.
- `url` — WS URL construction.
- `serde` + `serde_json` + `serde_repr` — packet codec. Uses `#[serde(tag = "packet")]` with numeric discriminator — see protocol notes below for why we avoid the default tagged-enum shape.
- `rfd` — native file dialogs (import/export).
- `directories` — cross-platform config dir.
- `rand` — token generation.
- `anyhow` / `thiserror` — errors.
- `similar` — diff for `FileAction.Patch`.
- `log` + `env_logger` — logging.

---

## Protocol reference (cribbed from `src/network.ts` and `src/server/index.ts`)

### URL
```
wss://cc.minecartchris.cc/connect?id=<token>&capabilities=file:edit
```
(Use `ws://` for localhost.) For file editing only, capability set is just `file:edit`. If we later add terminal viewing, append `,terminal:view`.

### Token
32 chars, `[A-Za-z0-9]`. `checkToken` == exact length 32, alnum only.

### Packet codes (`PacketCode`, numeric)
```
0x00 ConnectionUpdate   {packet, clients, capabilities:[string]}
0x01 ConnectionAbuse    {packet, message}
0x02 ConnectionPing     {packet}   // echo back identically to keep-alive
0x10 TerminalContents   {packet, width, height, cursorX, cursorY, cursorBlink, palette, text:[], fore:[], back:[]}
0x11 TerminalEvents     {packet, events:[{name, args:[]}]}
0x12 TerminalInfo       {packet, id, label?}
0x20 FileListing        {packet, id, files:[{file, checksum}]}
0x21 FileRequest        {packet, id, file:[{file, checksum}]}
0x22 FileAction         {packet, id, actions:[FileActionEntry]}
0x23 FileConsume        {packet, id, files:[{file, checksum, result}]}
```

`FileActionEntry` = `{file, checksum, flags}` plus one of:
- `action=0 Replace`  → extra `{contents}`
- `action=1 Patch`    → extra `{delta:[Fragment]}`
- `action=2 Delete`   → no extra fields

`FileActionFlags` bitmask: `ReadOnly=1, Force=2, Open=4, New=8`.

`FileConsume` result enum: `OK=1, Reject=2, Failure=3`.

`Fragment` char-level diff:
- `{kind:0, length}` Same
- `{kind:1, contents}` Added
- `{kind:2, length}` Removed

### Checksum
Fletcher32 over the file as JS string (UTF-16 code units). The CC client computes it on UTF-8 bytes — both produce matching values for ASCII-only Lua, which is the common case. For v1 we implement Fletcher32 on UTF-8 bytes; if users report checksum mismatches on non-ASCII files, revisit.

Reference impl (`src/viewer/packet.ts`):
```ts
let s1=0, s2=0;
if (contents.length % 2 !== 0) contents += "\0";
for (let i=0; i<contents.length; i+=2) {
  const c1 = contents.charCodeAt(i), c2 = contents.charCodeAt(i+1);
  s1 = (s1 + c1 + (c2 << 8)) % 0xFFFF;
  s2 = (s1 + s2) % 0xFFFF;
}
return (s2 << 16 | s1) >>> 0;
```

### Save flow (client → server)
1. User edits buffer. On Ctrl+S:
2. Compute `new_checksum = fletcher32(contents)`.
3. Compute `delta = diff(remote_contents, contents)` (char-level).
4. Send `FileAction{ id: 0, actions: [{file, checksum: remote_checksum, flags: 0, action: Patch, delta}] }`. Use `Replace` if `isNew` was true.
5. Track `update_checksum = new_checksum`, `update_contents = contents`.
6. On `FileConsume`:
   - `OK` and matching `update_checksum` → mark saved, set `remote_contents/checksum = update_*`.
   - `Reject` → show "remote changed" warning.
   - `Failure` → show "could not save" error.

### Open flow (client → us)
Server sends `FileAction.Replace` with flag `Open (4)` when the user runs `samari edit <file>` in-game. Behavior: show file in editor, select it active.

### Connection state machine
- `ConnectionUpdate` arrives whenever any peer connects/disconnects. `capabilities` is the union of OTHER peers' capabilities.
- Before we've ever seen a host connect (`terminal:host` or `file:host` in caps), show a "waiting" screen with the Lua bootstrap command (for our app: "run `samari.lua <token>` in-game to connect").
- If the host was connected and leaves, show "connection lost".

### Keep-alive
Server sends `ConnectionPing` every 15s. Echo back identically or the server will terminate.

---

## Task checklist

Legend: `[x]` done, `[~]` in progress, `[ ]` todo.

### Foundation
- [x] Decide scope (file editor first, terminal stubbed).
- [x] PLAN.md.
- [x] `Cargo.toml` with crate list above.
- [x] Module skeleton files under `src/` with stubs.
- [x] `main.rs` → `eframe::run_native` opens window.

### Token + checksum + diff
- [x] `token.rs`: `Token` newtype, `gen_token()`, `check_token(&str)`, `fletcher32(&str)`. (Unit tests in-file.)
- [x] `diff_patch.rs`: `Fragment` enum, `compute_diff(old, new)` via `similar::TextDiff::diff_chars`. No `apply_patch` — server→us is always `Replace`, so we never need to apply patches client-side.

### Protocol
- [x] `protocol.rs`: `PacketCode` enum, manual `encode` / `decode` via `serde_json::Value` intermediate. Handles all 10 packet types; round-trip tests cover ping + connection update + unknown code. `TerminalContents` is parsed-but-preserved as `Value` (not used by the file editor).

### Session task
- [x] `session.rs`: `Session` struct, spawn via `tokio::runtime::Handle`, std::mpsc inbound / tokio::mpsc outbound. Handles `ConnectionUpdate` state machine, echoes `ConnectionPing`, applies `FileAction` (Replace + Delete; Patch from server not yet supported — CC host never sends it), and `FileConsume`. Has `save_file` / `save_active` that computes the diff, tracks `pending_update`, and sends `FileAction.Patch` (or `Replace` if `is_new`).

### Storage
- [x] `storage.rs`: `ProjectDirs("cc","minecartchris","samari-catcher")` → `config.json`. Atomic write via tmp + rename. `import_tokens` / `export_tokens` for JSON `{tokens:[...]}` bundles.

### UI
- [x] `app.rs`: `AppState`, `eframe::App::update` drains sessions, lays out tab bar + view switcher + central panel. Persists on change and on exit. Import/export via `rfd::FileDialog`.
- [x] `ui/tabs.rs`: tab strip with add/close/select/+token/import/export/settings. Status dot per tab (yellow=connecting, orange=waiting, green=connected, red=errored/lost).
- [x] `ui/files.rs`: left sidebar + central editor. Ctrl+S saves (also a button). Notifications banner for save warnings/errors.
- [x] `ui/terminal.rs`: stub with hyperlink to `https://cc.minecartchris.cc/?id=<token>`.
- [x] `ui/settings.rs`: dark mode, font size slider, trim-on-save, server host field.

### Smoke test
- [x] `cargo build` and `cargo test` pass clean (3 warnings about unused helper APIs — left in for future callers).
- [ ] **End-to-end:** start the node server (`CLOUD_CATCHER_PORT=8080 npm run host` in the repo root), set `SAMARI_DEV=1` and `cargo run`, attach a CC computer with `samari.lua <token>`, verify: file appears in list, edits save, reopen preserves tabs. *Not done — requires a live CC client.*

### Next-up (not started — for the continuing AI)
- [ ] **Terminal view.** Replace `ui/terminal.rs` stub with a real renderer. Reference: `src/viewer/computer/index.tsx` for how `TerminalContents` packets drive a `TerminalData` grid, and `node_modules/@squid-dev/cc-web-term/dist/terminal.*` for the canvas-based renderer's math (character cell size, bitmap font atlas, palette). Simplest path in egui: render a `TextureHandle` of the packed terminal grid using a monospace font and per-cell foreground/background rects.
- [ ] **Terminal input.** Add `Capability::TerminalView` to the connect URL (it's currently file:edit only). Map egui `Key` → CC key names (port `keyName()` from cc-web-term — mostly a big match statement). Send `Packet::TerminalEvents` on keydown/keyup.
- [ ] **Lua syntax highlighting.** Drop in `syntect` with the Lua `.sublime-syntax`. egui's `TextEdit` can take a `LayoutJob` via a layouter closure — that's the hook.
- [ ] **Patch applier.** If the server ever starts sending `FileAction.Patch` our way, we need to walk `delta` and rebuild the file. See `src/host/init.lua` lines ~391–425 for the reference algorithm.
- [ ] **Checksum non-ASCII parity.** See "Known gotchas / decisions" below. Test against files with `é` / `日本語` and, if it fails, make our fletcher32 run on UTF-16 code units like the web client does.
- [ ] **Per-tab connection toggles.** Right now every tab auto-connects; we might want a "pause" button to keep a tab saved without an open WS. Not urgent.
- [ ] **Icon + installer.** Bundle an icon. `cargo install cargo-bundle` or `tauri bundle` can produce `.msi`/`.dmg`. Skipped for v1.

---

## Known gotchas / decisions

- **rustls crypto provider.** `rustls` ≥ 0.23 stopped auto-picking a provider. We enable `rustls = { features = ["ring"] }` in `Cargo.toml` and call `rustls::crypto::ring::default_provider().install_default()` once at the top of `main.rs`. Without this, the WS task panics on first TLS handshake ("Could not automatically determine the process-level CryptoProvider"). If the panic ever returns, check both the `Cargo.toml` feature and the `install_default()` call — it must run before `connect_async` on the tokio task.
- **egui + tokio.** `eframe` runs its own event loop; we can't just `#[tokio::main]`. Create a `tokio::runtime::Runtime` inside `main.rs` and spawn sessions onto it with `rt.spawn(...)`. Hold the runtime as an `Arc<Runtime>` passed into `Session::spawn`.
- **Channel back-pressure from async → sync.** Use `std::sync::mpsc` (not `tokio::sync::mpsc`) for inbound because egui polls synchronously each frame. Use `tokio::sync::mpsc::UnboundedSender<String>` for outbound (UI → task) since `send` is sync-callable.
- **Repaint.** Clone `egui::Context` into the session task; call `ctx.request_repaint()` after pushing to the inbound channel. Without this, the UI won't wake until the user moves the mouse.
- **Packet discriminator.** `serde_repr` + `#[serde(tag)]` isn't supported directly in stable serde. Do a manual Deserialize impl that reads the `"packet"` field as u8, then dispatches. Easier to debug than wrangling macros.
- **Diff size.** For large files the char-level diff can blow past `MAX_PACKET_SIZE = 20_000` bytes. The web client doesn't guard against this; we should log a warning and fall back to `Replace` if the serialized patch exceeds ~16KB.
- **Settings key parity with web.** The web client stores settings in `window.localStorage.settings` as JSON. We don't need byte-for-byte parity; our config lives in `ProjectDirs.config_dir()/config.json`. Don't try to read the browser's localStorage.
- **File paths with leading slash.** The display format in the web UI strips/re-adds `/`. Keep the canonical stored name as whatever the server sends; only prettify for display.
- **Read-only files.** Flag 1 means read-only. Don't send save packets; show a banner.
- **Editor undo across saves.** The web UI tracks `savedVersionId` from Monaco's alternative version id. egui's `TextEdit` has no such concept — just track "buffer == remote_contents" for the dirty flag. Good enough for v1.

---

## Resume / handoff checklist (next AI — read this if continuing)

1. Read this file top to bottom.
2. Look at task checklist — continue from first `[ ]`.
3. For protocol questions, cross-reference `../src/network.ts` (TS types) and `../src/server/index.ts` (what the server accepts / forwards).
4. For file-editing semantics, cross-reference `../src/viewer/computer/index.tsx` (how the web client handles `FileAction`, `FileConsume`, dirty tracking, notifications).
5. Local dev: `CLOUD_CATCHER_PORT=8080 npm run host` in the repo root, then in `desktop/`: `SAMARI_DEV=1 cargo run`.
6. Remote prod: `wss://cc.minecartchris.cc/connect?...` — requires the server at 127.0.0.1:8080 on 192.168.1.242 (see nginx config already in place; see project memory for SSH details).
7. Once file editing feels solid and the user approves, start on the terminal — see `../src/viewer/computer/index.tsx` (`TerminalData` in `cc-web-term`) and `../src/host/framebuffer.lua` for the wire format.

---

## Milestones

- [x] **M1:** Cargo.toml + module skeletons + empty window runs.
- [x] **M2:** Token/checksum/diff + protocol codec with round-trip tests.
- [x] **M3:** WS session task spawns, handshake, echoes pings, surfaces status in UI.
- [x] **M4:** File list + editor + Ctrl+S save code path. *End-to-end verification against a live CC computer still pending.*
- [x] **M5:** Tabs + persistence + import/export.
- [x] **M6:** Settings dialog + dark mode.

Current state: everything compiles, tests pass, the binary is at `desktop/target/debug/samari-catcher-desktop.exe`. Run once with a real token attached to verify; after that, start on the terminal view (see "Next-up").

## Verified build / test commands

```
cd desktop
cargo test           # runs 9 tests, all pass
cargo build          # produces target/debug/samari-catcher-desktop[.exe]
cargo run            # launches the app against cc.minecartchris.cc
SAMARI_DEV=1 cargo run   # launches against ws://localhost:8080
```
