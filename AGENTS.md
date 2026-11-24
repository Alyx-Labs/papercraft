# AGENTS.md: PaperCraft Architectural Rules

PaperCraft is a cross-platform note-taking and handwriting application written in **pure Rust**, following a strict layered clean-room architecture.

## Current Scope

- **Drawing, Handwriting, Documents, Notebooks, Sync & Export Engine**: fountain/ball/brush/pencil/highlighter/eraser engines (including highlighter-only eraser mode), hardware stylus pressure/tilt/barrel-button/eraser-end support, stroke stabilization (1€ adaptive filter, Pulled-String, Exponential Moving Average, Gaussian window, StreamLine fairing, Circular Buffer arithmetic/Gaussian smoothing), Centripetal Catmull-Rom splines, kinematic tip prediction, freehand Draw & Hold shape recognition, positioned rich text boxes (formatting, lists, checkboxes, links), positioned raster images (file picker, clipboard paste, drag-and-drop), built-in stickers, geometric shapes (fill & border styles), rectangular & freeform lasso selection/transform with cross-page copy/paste, ruler, laser pointer, PDF import on a protected background layer, page size & orientation picker, built-in and custom paper templates, multi-page notebooks with folders/favorites/tags/bookmarks/batch actions, multi-format export (`.png`, `.svg`, `.pdf`, `.pcraft`, `.pcraftpkg`), optional self-hosted sync server (`papercraft-server-cli` CLI and `papercraft-server` system-tray control app), and client-side WebAssembly browser app (`papercraft-web`).
- **Alpha focus**: Keep the workspace extremely lightweight while validating the core concept with the community on Desktop and Web ahead of future mobile ports (Android/iOS).

## Workspace Layers (`cargo xtask layers`)

- **L0 Foundation chain**: `papercraft-geom` -> `papercraft-color` -> `papercraft-raster`
- **L0 Standalone**: `papercraft-tablet` (no workspace deps; the only crate permitting localized `unsafe` for OS pointer/tablet hooks)
- **L1 Document**: `papercraft-doc` (pure serializable data; large point buffers `Arc`-shared)
- **L2 Operations, Paint & Sync Protocol**: `papercraft-ops`, `papercraft-paint`, `papercraft-sync-protocol`
- **L3 Compositor & Format**: `papercraft-compose` (CPU oracle), `papercraft-format` (`.pcraft` JSON/binary format)
- **L4 Sync Client & Updater**: `papercraft-sync`, `papercraft-updater`
- **L5 Engine**: `papercraft-engine` (headless `Session` + command registry; every action is a command)
- **L6 UI Shell**: `papercraft-ui-egui` (thin `egui`/`eframe` shell; no UI crates permitted below L6)
- **Apps**: `papercraft` (desktop), `papercraft-server` (system-tray sync server control app), `papercraft-server-cli` (self-hosted CLI sync server), `papercraft-web` (WebAssembly), `papercraft-cli` (headless CLI), `xtask`

## Common Commands

- **Run desktop app**: `cargo run -p papercraft`
- **Run server tray app**: `cargo run -p papercraft-server`
- **Run web app (WebAssembly)**: `cargo xtask web` (or `cd apps/papercraft-web && trunk serve`), then open `http://127.0.0.1:8080` (prerequisites: `rustup target add wasm32-unknown-unknown` and `cargo install --locked trunk`)
- **Build web release bundle (`dist/`)**: `cargo xtask web-build` (or `cd apps/papercraft-web && trunk build --release`)
- **Run sync server**: `cargo run -p papercraft-server-cli -- serve --bind 127.0.0.1:8787`
- **Run CLI**: `cargo run -p papercraft-cli -- commands`
- **Local auto-update test server**: `cargo xtask update-dev-server --version 0.2.0` (serves local binaries and `latest.json` on `http://127.0.0.1:8799/latest.json` for safe testing via `PAPERCRAFT_UPDATE_MANIFEST_URL` and `PAPERCRAFT_UPDATE_CURRENT_VERSION`)
- **Verify layers & CI**: `cargo xtask layers` / `cargo xtask ci`

## Golden Rules

1. **Never crash**: Non-test code must never use `unwrap()`, `expect()`, `panic!()`, `todo!()`, `unimplemented!()`, or `unreachable!()`.
2. **No `unsafe`**: Workspace forbids `unsafe_code`; only `papercraft-tablet` platform interop modules allow localized `unsafe` with `SAFETY:` comments.
3. **Hostile inputs**: Sanitize `NaN`/`Infinity`, guard division by zero, bound allocations and recursion.
4. **Everything is a command**: All user-visible document/tool mutations dispatch through `papercraft-engine` commands and must pass `panic_hunt`.
5. **No third-party app names**: Never mention external applications in code identifiers, comments, docstrings, metadata, or UI strings.

## Verification Workflow

1. **Finish sequential edits before heavy checks**: When handling multiple requests or queued follow-ups in sequence, keep applying all requested edits first; light, scoped checks (`cargo check -p <crate>` or a focused crate test) are fine when needed, while heavy workspace-wide commands wait until no pending requests remain.
2. **Use scoped checks for localized changes**: For small edits contained within one crate, verify with targeted checks (`cargo check -p <crate>`, `cargo test -p <crate>`) rather than running the full workspace suite.
3. **Save heavy CI commands for the final step**: Run `cargo xtask ci` (plus `wasm32-unknown-unknown` `check` and `clippy` for `papercraft-web` or shared UI/engine changes) only once at the very end when no further requests remain in the batch, and always before opening a PR, pushing `main`, or tagging a release (matching `.github/workflows/ci.yml`).

## UI & UX Design Rules

1. **100% English UI**: Every user-visible string—button labels, tooltips, presets, explanations, dialogs, and status text—must be written in clear, concise English.
2. **Lucide SVG Icons**: Every tool and action uses a locally vendored outline SVG from Lucide (`crates/ui-egui/assets/icons/lucide/`), rendered through the semantic `Icon` registry in `crates/ui-egui/src/icons.rs`. Keep the set pinned to its documented version, preserve the upstream ISC license, use rounded 24×24 outline artwork, and tint icons from the active theme. Add only the SVGs needed by the app and update `docs/ICONOGRAPHY.md`; do not draw replacement icons directly with `egui::Painter` or introduce another icon library.
3. **3-Zone Uncluttered Toolbar, Left Pages Sidebar, Right Settings Drawer & Standard Modals**: The top toolbar is organized into three balanced zones (Left: Library, compact Notebook & page switcher; Center: writing tools, thickness dots, customizable quick color swatches & free HSV/RGB/Hex color picker popover; Right: undo/redo/clear, zoom, export modal toggle, and **Settings** toggle `S`), with no pen preset selector dropdown in the toolbar; the floating pen context tray includes a compact per-pen **Smoothing** pill (`Off` / `%`) opening a minimal percentage-only popover (`Off / Low / Med / High` + `0%–100%` strength slider). The left sidebar (`P`) keeps open notebook switching compact and centers on clean book-style visual page thumbnails; the right-hand Settings studio houses three tabs (`Pen` for stylus detection, hardware button shortcuts, and active pen reset; `Appearance`; `Telemetry`); and multi-step customization and file workflows (`New Notebook`, `Notebook Settings`, `Page & Paper Setup`, and `Export`) open through a unified centered modal architecture (`crates/ui-egui/src/modal.rs`).
4. **Arithmetic Circular-Buffer Handwriting Stabilization (Per Pen)**: Every writing pen uses the classic arithmetic mean circular-buffer stabilizer with C1 tail finalization (`ArithmeticBuffer10`, default `50%` / window `10`; `0%` maps to direct 1:1 tracking and `1%–100%` scales up to window `20`) adjusted exclusively via the floating pen context tray's percentage smoothing popover (`Off / Low / Med / High` + `0%–100%` slider) without exposing buffer counts or raw parameters in the UI. No smoothing preset/style cards, raw algorithm parameter tuning, or smoothing controls live in the Settings sidebar.
5. **Plain-English Inline Explanations**: Every setting card, preset, and control must include an inline, non-jargon subtitle explaining *what it adjusts* and *how it changes the feel of writing*.
6. **Clean Light & Dark Studio Themes**: Follow `crates/ui-egui/src/theme.rs` tokens (`UiTheme`, `Tokens::light()`, `Tokens::dark()`, `Tokens::for_theme`, `canvas_bg`, `toolbar_bg`, `panel_bg`, `field_bg`, `border`, `accent`, `accent_hover`, `accent_soft`, `text`, `text_muted`, `danger_fg`/`danger_border`/`danger_bg`) with rounded card containers (`section_card_header`, `card_frame`), generous spacing, and user-selectable Light/Dark switching via `theme.set` persisted in `studio_prefs.json`.
7. **Minimalist Canvas Controls & Percentage-Only Per-Pen Smoothing**: In the floating pen context tray, expose only quick 1-click thickness dots/slider and a compact per-pen smoothing pill (`Off` or `%` opening an `Off / Low / Med / High` + `0%–100%` slider popover backed by the arithmetic circular buffer up to window `20`, default `50%` = `10`) persisted independently per pen in `UserStudioConfig` (`studio_prefs.json` / Web `localStorage` / Sync), with NO preset dropdown, NO style selector, NO smoothing controls in the Settings sidebar, and NO raw algorithm parameter or buffer-size controls.

## Release & Auto-Update Protocol

1. **Always interview the user before releasing**: Whenever the user asks to release, publish, deploy, or bump a version—or before pushing `main` or creating/pushing any `v*` tag—ALWAYS ask the user first how they want to proceed, covering:
   - **Release channel**:
     - `dev` only (save work in progress; no Web deploy, no GitHub Release, and no Desktop/Server auto-update);
     - `main` only without a tag (triggers `.github/workflows/pages.yml` to update the Web app at `web.papercraft.ink`, without triggering Desktop/Server auto-update);
     - **Pre-release tag** (`vX.Y.Z-rc.1` or any `v*` tag containing `-`): triggers `.github/workflows/release.yml` to build real binaries marked as a GitHub **Pre-release**, without changing `/releases/latest/download/latest.json` or notifying stable users;
     - **Public production release** (stable `vX.Y.Z` tag): fast-forwards `main` from `dev`, pushes `main`, and pushes tag `vX.Y.Z`, triggering `.github/workflows/release.yml` to publish binaries, `SHA256SUMS.txt`, and `latest.json` (automatically notifies installed Desktop and Server users).
   - **Target version number** (patch, minor, or pre-release suffix).
   - **Execution preference**: whether the agent should run the git commit/merge/tag/push commands or only prepare and verify the files locally.
2. **Never push `main` or tags without explicit confirmation**: Never push to `main` or create/push a `v*` tag unless the user has explicitly selected that channel and confirmed.
3. **Keep version strings in sync with the tag**: Before creating any `vX.Y.Z` (or `vX.Y.Z-rc.N`) tag, update `[workspace.package] version` in `Cargo.toml` to match the tag version (preventing an infinite update loop where the newly downloaded binary still reports the old version), refresh `Cargo.lock`, update the macOS fallback `APP_VER` in `.github/workflows/release.yml`, and (for stable releases) update the version references in `README.md`. Never modify external dependency versions such as `winres = "0.1.12"`.
4. **Pre-release verification**: Before pushing `main` or any release tag, run `cargo xtask ci`, `cargo check --target wasm32-unknown-unknown -p papercraft-web --all-targets`, and `cargo clippy --target wasm32-unknown-unknown -p papercraft-web --all-targets --no-deps -- -D warnings`.
