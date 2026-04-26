# Samurai Catcher — Contribution Guidelines

This is the project-wide cheat sheet for working in this repo. It covers both the
web client (TypeScript / Preact / Lua) at the root and the native desktop app
(Rust / `egui` / `tokio`) under [desktop/](desktop/). If you're an AI assistant
or a new human contributor, read this first, then `PLAN.md` and `desktop/PLAN.md`
for the deeper architectural narrative.

---

## Repository layout

| Path | What lives here |
|------|-----------------|
| [src/](src/) | Web client + Node relay server. TypeScript + Preact (viewer/editor), TypeScript (server), Lua (CC host bootstrap). |
| [desktop/](desktop/) | Native Rust desktop client (`samari-catcher-desktop`). |
| [public/](public/) | Static files for the web build. |
| [tools/](tools/) | Build helpers (Rollup setup, Lua bundler). |
| [_bin/](_bin/) | Compiled Node binary entry point. |
| [_site/](_site/) | Built web assets served by the relay. |
| [PLAN.md](PLAN.md) | High-level plan for the whole project. |
| [desktop/PLAN.md](desktop/PLAN.md) | Deep dive on the Rust desktop app. |

---

## Code style

### Rust (desktop crate)

- `cargo fmt` before committing — no exceptions.
- `cargo clippy --all-targets` — fix warnings, don't `#[allow]` unless documented.
- Prefer `anyhow::Result` for app code, `thiserror`-derived errors only for crate-public types.
- Module docs at the top of each file. One short paragraph explaining what it does and how it's used.
- No comments that just restate the code. Comments belong on hidden constraints, gotchas, and "why not the obvious approach".
- Keep `unsafe` out unless absolutely necessary; today there is none.
- The project is a `cargo workspace` rooted at the top-level [Cargo.toml](Cargo.toml). The `desktop` crate is the only member.

### TypeScript / Preact (web client)

- Run the existing ESLint config: `npm run lint`. Fix issues with `npm run lint:fix`.
- Match the existing style: 2-space indent, double quotes, semicolons.
- No new dependencies without first checking they aren't already pulled in transitively.

### Lua (host bootstrap)

- Keep `.luacheckrc` warnings clean.
- Tabs are 2 spaces. Match the surrounding file.

### General

- Don't commit build artifacts (`target/`, `_site/`, `_bin/`) unless they're
  already tracked and intended to be shipped.
- Never commit secrets or `.env` files.
- One change per commit. Use short imperative subjects: "fix terminal arrow keys",
  not "Fixing arrow keys (please work this time)".

---

## Architecture rules of thumb

### Web client (`src/`)

- The viewer and editor talk to the relay over a single WebSocket. The wire
  protocol is documented in [PLAN.md](PLAN.md) ("Protocol reference") and
  reflected in [src/network.ts](src/network.ts).
- The relay only forwards packets between paired peers; it does not interpret
  payloads. Don't push business logic into the relay.
- The CC bootstrap (Lua) lives in `src/host/`. Keep it small and dependency-free
  — it has to fit in a CC computer's RAM.

### Desktop client (`desktop/`)

- The UI thread runs `eframe` / `egui`. Do **not** block it. Async work goes on
  the shared `tokio::runtime::Runtime` owned by `main.rs`.
- One WebSocket task per open tab. UI ↔ task communication uses
  `std::sync::mpsc` for inbound (sync-pollable from `egui::App::update`) and
  `tokio::sync::mpsc::UnboundedSender<String>` for outbound.
- Always `egui::Context::request_repaint()` from a background task after pushing
  to the inbound channel — otherwise the UI sleeps until the user moves the
  mouse.
- All new HTTP clients (Ollama agent, etc.) follow the same pattern: a tokio
  task + an inbound channel + a `request_repaint()`.

### Shared

- The wire protocol is canonical in TypeScript ([src/network.ts](src/network.ts))
  and re-implemented in Rust ([desktop/src/protocol.rs](desktop/src/protocol.rs)).
  When changing it, update both sides and keep packet codes byte-for-byte
  identical.
- Tokens are 32 chars `[A-Za-z0-9]`. Don't shorten — the relay rejects malformed
  ones.

---

## Building

### Web client

```sh
npm install
npm run build       # full build (TS + Lua + setup)
npm run host        # run the relay locally on port 8080
npm run lint        # eslint
```

### Desktop client

```sh
cd desktop
cargo build
cargo test
cargo run                   # connects to cc.minecartchris.cc by default
SAMARI_DEV=1 cargo run      # connects to ws://localhost:8080
```

### Local end-to-end smoke test

1. `CLOUD_CATCHER_PORT=8080 npm run host` in the repo root.
2. In another terminal: `cd desktop && SAMARI_DEV=1 cargo run`.
3. Click `+` to add a session. Copy the token.
4. Run `samari.lua <token>` in a CC computer to attach.
5. Try `samari edit hello.lua` on the computer — the file should open in the
   desktop app's editor.

---

## Adding a feature

1. Read the relevant `PLAN.md`. It's the source of truth for in-scope vs
   out-of-scope.
2. If your change touches the wire protocol, update both `src/network.ts` and
   `desktop/src/protocol.rs`. Add a round-trip test in `desktop/src/protocol.rs`.
3. If your change adds settings, update both `Settings` in
   [desktop/src/settings.rs](desktop/src/settings.rs) and the settings UI in
   [desktop/src/ui/settings.rs](desktop/src/ui/settings.rs). Defaults must be
   sensible — users will never click "reset".
4. If your change spawns a new background task, plumb a repaint call.
5. Add or update a test where it's cheap to do so. The desktop crate already
   has unit tests for tokens, diff, and protocol; mirror that style.

---

## Keyboard input (desktop)

Both the file editor and the terminal handle keys; their requirements differ:

- **File editor (Files view)**: `egui::TextEdit::multiline` handles Enter,
  Backspace, and arrow keys natively. The custom layouter for syntax
  highlighting must produce a `LayoutJob` whose `text` exactly matches the
  buffer — otherwise cursor positioning breaks. Don't strip newlines.
- **Terminal view**: keys are forwarded to the CC computer as
  `cloud_catcher_key` / `cloud_catcher_key_up` / `char` events. The terminal
  area must `request_focus()` while it is the active view, and must drain key
  events from `ctx.input(...)` so other widgets don't intercept them. See the
  reference in [src/viewer/computer/index.tsx](src/viewer/computer/index.tsx)
  and the key-name table in
  [node_modules/@squid-dev/cc-web-term/dist/terminal/input.js](node_modules/@squid-dev/cc-web-term/dist/terminal/input.js).

---

## AI agent (Ollama)

The desktop app supports an in-app AI editor backed by a local Ollama server.

- Default endpoint is `http://localhost:11434`. Default model is whatever the
  user configures; recommended choices are `qwen2.5-coder:7b` or
  `deepseek-coder:6.7b` for code work.
- Requests are non-streaming (`stream:false`) — keep it simple, the editor
  applies the whole response at once.
- The agent runs in a tokio task and reports back to the UI via
  `std::sync::mpsc`. It must call `egui::Context::request_repaint()` on
  completion.
- Never auto-apply AI output to the buffer. The user always clicks "Apply".
- Don't send arbitrary file contents to remote servers. Local Ollama only.

---

## Releasing the desktop app

1. Bump the version in `desktop/Cargo.toml`.
2. `cargo build --release` produces `desktop/target/release/samari-catcher-desktop[.exe]`.
3. The release binary is statically linked enough to copy onto another machine
   of the same OS / arch.
4. There is no installer or auto-updater yet — see "Out of scope" in
   [desktop/PLAN.md](desktop/PLAN.md).

---

## Things the AI assistant should NOT do without asking

- Push to `main` or any shared remote branch.
- Touch `flake.nix` / `flake.lock` — those are tied to the maintainer's build
  environment.
- Add new top-level dependencies (npm or cargo) on a whim. Justify in the PR
  description.
- Rewrite `PLAN.md` files from scratch. They're append-only logs; preserve
  history.
- Bypass `eslint` / `clippy` / `cargo fmt` errors with `--no-verify` or `#[allow]`.
- Modify the WebSocket relay to interpret payloads — it's intentionally dumb.
