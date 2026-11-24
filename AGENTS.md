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
- **L4 Sync Client**: `papercraft-sync`
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
- **Verify layers & CI**: `cargo xtask layers` / `cargo xtask ci`

## Golden Rules

1. **Never crash**: Non-test code must never use `unwrap()`, `expect()`, `panic!()`, `todo!()`, `unimplemented!()`, or `unreachable!()`.
2. **No `unsafe`**: Workspace forbids `unsafe_code`; only `papercraft-tablet` platform interop modules allow localized `unsafe` with `SAFETY:` comments.
3. **Hostile inputs**: Sanitize `NaN`/`Infinity`, guard division by zero, bound allocations and recursion.
4. **Everything is a command**: All user-visible document/tool mutations dispatch through `papercraft-engine` commands and must pass `panic_hunt`.
5. **No third-party app names**: Never mention external applications in code identifiers, comments, docstrings, metadata, or UI strings.

## UI & UX Design Rules

1. **100% English UI**: Every user-visible string—button labels, tooltips, presets, explanations, dialogs, and status text—must be written in clear, concise English.
2. **Lucide SVG Icons**: Every tool and action uses a locally vendored outline SVG from Lucide (`crates/ui-egui/assets/icons/lucide/`), rendered through the semantic `Icon` registry in `crates/ui-egui/src/icons.rs`. Keep the set pinned to its documented version, preserve the upstream ISC license, use rounded 24×24 outline artwork, and tint icons from the active theme. Add only the SVGs needed by the app and update `docs/ICONOGRAPHY.md`; do not draw replacement icons directly with `egui::Painter` or introduce another icon library.
3. **3-Zone Uncluttered Toolbar, Left Pages Sidebar, Right Settings Drawer & Standard Modals**: The top toolbar is organized into three balanced zones (Left: Library, compact Notebook & page switcher; Center: writing tools, thickness dots, customizable quick color swatches & free HSV/RGB/Hex color picker popover; Right: undo/redo/clear, zoom, export modal toggle, and **Settings** toggle `S`), with no pen preset selector dropdown in the toolbar; the floating pen context tray includes a compact **Smoothing** pill (`Off` / `%`) opening a minimal popover (`Off / Low / Med / High` + `0%–100%` strength slider). The left sidebar (`P`) keeps open notebook switching compact and centers on clean book-style visual page thumbnails; the right-hand Settings studio houses three tabs (`Pen`, `Appearance`, `Telemetry`); and multi-step customization and file workflows (`New Notebook`, `Notebook Settings`, `Page & Paper Setup`, and `Export`) open through a unified centered modal architecture (`crates/ui-egui/src/modal.rs`).
4. **Unified Smoothing Strength & 4 Smoothing Styles with Hybrid Live Preview**: Settings -> `Pen` offers the `0%–100%` smoothing strength slider (`Off / Low / Med / High` quick levels) alongside 4 curated smoothing styles (`StabilizationPreset::UI_STYLES`: `Adaptive`, `ModeledSpring`, `CurvedBezier`, `ArithmeticBuffer10`) and a real-time rendered stroke preview combining a slow tremor curve with fast sharp-corner turns, changing only the smoothing of the active pen without exposing raw algorithm parameter tuning or overriding toolbar size/color.
5. **Plain-English Inline Explanations**: Every setting card, preset, and control must include an inline, non-jargon subtitle explaining *what it adjusts* and *how it changes the feel of writing*.
6. **Clean Light & Dark Studio Themes**: Follow `crates/ui-egui/src/theme.rs` tokens (`UiTheme`, `Tokens::light()`, `Tokens::dark()`, `Tokens::for_theme`, `canvas_bg`, `toolbar_bg`, `panel_bg`, `field_bg`, `border`, `accent`, `accent_hover`, `accent_soft`, `text`, `text_muted`, `danger_fg`/`danger_border`/`danger_bg`) with rounded card containers (`section_card_header`, `card_frame`), generous spacing, and user-selectable Light/Dark switching via `theme.set` persisted in `studio_prefs.json`.
7. **Minimalist Canvas Controls & Adjustable Pen Smoothing**: In the floating pen context tray, expose only quick 1-click thickness dots/slider and a compact smoothing pill (`Off` or `%` popover with `0%–100%` strength), with NO preset dropdown. In the right-hand Settings drawer, `Pen` exposes the unified smoothing strength control (`0%–100%` + `Off / Low / Med / High`) and 4 curated 1-click smoothing styles (`Adaptive` by default, `ModeledSpring`, `CurvedBezier`, `ArithmeticBuffer10`) persisted per pen in `UserStudioConfig` (`studio_prefs.json` / Web `localStorage` / Sync), with NO custom preset builder, NO built-in size/nib presets, and NO raw algorithm parameter controls.
