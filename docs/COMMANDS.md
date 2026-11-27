# PaperCraft Command Reference

A structured guide to running PaperCraft in **development mode**, generating **Windows / Desktop, Web, and Server builds**, and using the optional **CLI and verification tools**.

> Run all commands below from the **repository root** (your cloned `PaperCraft` folder) unless stated otherwise.

---

## Quick Cheat Sheet

### Run in Development Mode

| What you want to run | Command | Where it opens / listens |
|---|---|---|
| **Windows / Desktop app** | `cargo run -p papercraft` | Native desktop window |
| **Server Tray app (GUI)** | `cargo run -p papercraft-server` | System tray icon & minimalist server control window |
| **Web app (WebAssembly)** | `cargo xtask web` | Browser at `http://127.0.0.1:8080` (live reload) |
| **Sync Server (CLI)** (optional) | `cargo run -p papercraft-server-cli -- serve --data-dir target/dev-server-data --bind 127.0.0.1:8787` | HTTP API at `http://127.0.0.1:8787` |

### Generate Production / Release Builds

| What you want to build | Command | Output generated |
|---|---|---|
| **Windows Desktop `.exe`** | `cargo build --release -p papercraft` | `target\release\papercraft.exe` |
| **Windows Server Tray `.exe`** | `cargo build --release -p papercraft-server` | `target\release\papercraft-server.exe` |
| **Windows Sync Server CLI `.exe`** | `cargo build --release -p papercraft-server-cli` | `target\release\papercraft-server-cli.exe` |
| **Windows CLI `.exe`** | `cargo build --release -p papercraft-cli` | `target\release\papercraft-cli.exe` |
| **All 4 Windows `.exe` binaries at once** | `cargo build --release -p papercraft -p papercraft-server -p papercraft-server-cli -p papercraft-cli` | `target\release\*.exe` |
| **Web static production bundle** | `cargo xtask web-build` | `dist\` folder (`index.html`, `.js`, `.wasm`) |

---

## 1. Prerequisites

- **All targets (Desktop, Server, CLI):** Install the **Rust toolchain (1.95+)** via [rustup](https://rustup.rs/).
  - **Windows:** install the default `x86_64-pc-windows-msvc` toolchain (with the Visual Studio C++ Build Tools workload if prompted by `rustup`).
  - **Linux (Wayland & X11):** works out of the box with `winit` (`wayland-dlopen`) and `wgpu`. For native file dialogs and clipboard paste (`Ctrl+V` text & images), install `zenity` (or `kdialog`) plus `wl-clipboard` (Wayland) or `xclip`/`xsel` (X11).
  - **macOS (`aarch64` / Apple Silicon — experimental, needs community testing):** install Xcode Command Line Tools (`xcode-select --install`) and Rust via `rustup`. macOS builds compile in GitHub Actions, but have **not yet been verified on physical Mac hardware**—community testing and issue reports are welcome.
- **Web (WebAssembly) only (one-time setup):**
  ```bash
  rustup target add wasm32-unknown-unknown
  cargo install --locked trunk
  ```

---

## 2. Desktop App — Windows, Linux & macOS (`papercraft`)

The native desktop application is located in `apps/papercraft`.

### 2.1 Run in Development Mode

```bash
cargo run -p papercraft
```

- Compiles in the workspace `dev` profile (`opt-level = 1`, dependencies optimized at `opt-level = 2` for smooth ink rendering) and opens the PaperCraft window.
- Notebooks and settings are saved locally in a portable `library/` folder next to the executable (`<executable_dir>/library/`, e.g. `target/debug/library/` under `cargo run` or `target/release/library/` when running the release binary).

### 2.2 Generate the Release Build

To build the optimized standalone desktop executable on Windows, Linux, or macOS:

```bash
cargo build --release -p papercraft
```

What this generates:
- **Windows:** `target\release\papercraft.exe` (`apps/papercraft/build.rs` automatically embeds the `.ico` application icon and metadata, and hides the console window in release builds).
- **Linux & macOS:** `target/release/papercraft` (self-contained pure-Rust binary; macOS builds are currently experimental and need testing on real Mac hardware).

#### Build all desktop & server binaries together (like GitHub Releases)

If you want to produce the Desktop app, Server Tray app, Sync Server CLI, and CLI executables together:

```bash
cargo build --release -p papercraft -p papercraft-server -p papercraft-server-cli -p papercraft-cli
```

Or targeting an explicit triple (for example `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, or `aarch64-apple-darwin`):

```bash
cargo build --release --target x86_64-unknown-linux-gnu -p papercraft -p papercraft-server -p papercraft-server-cli -p papercraft-cli
```

### 2.3 Run the Compiled Binary

```bash
# Linux / macOS
./target/release/papercraft

# Windows (PowerShell)
.\target\release\papercraft.exe
```

You can also double-click `target\release\papercraft.exe` in Windows Explorer or copy the binary to any folder—it is a self-contained pure-Rust executable.

> [!TIP]
> **Choose a permanent folder before regular use:** Whether you downloaded `papercraft` from GitHub Releases or copied `target\release\papercraft` (Windows `.exe`, Linux `.AppImage`, or macOS `.app`), place it directly in the permanent folder where you want to keep it (for example `C:\Apps\PaperCraft\`, `~/Applications/PaperCraft/`, or `/Applications`). On Windows and Linux, notebooks live in `library/` right beside the executable, in-app updates replace the binary in place there, and the **Create Desktop shortcut automatically** switch (**Settings -> System -> App Updates**) creates an OS-native Desktop shortcut pointing to that location and refreshes it automatically whenever the app updates.

---

## 3. Web App (`papercraft-web` — WebAssembly)

The browser version is located in `apps/papercraft-web` (hosted live demo: **[https://web.papercraft.ink/](https://web.papercraft.ink/)**). It runs 100% client-side via WebAssembly and saves local data in browser `localStorage` (`papercraft_workspace_v1`).

> Make sure you ran the one-time Web prerequisites first (`rustup target add wasm32-unknown-unknown` and `cargo install --locked trunk`).

### 3.1 Run in Development Mode (Live Reload)

```powershell
cargo xtask web
```

Then open **`http://127.0.0.1:8080`** in your browser.

- `cargo xtask web` runs `trunk serve` inside `apps/papercraft-web`.
- It watches Rust and HTML files, recompiles automatically when you edit code, and reloads the browser tab.
- Equivalent manual command: `cd apps/papercraft-web && trunk serve`.

Useful development flags (forwarded directly to `trunk serve`):

```powershell
# Open the browser automatically once built
cargo xtask web --open

# Expose the dev server on your local network (for testing on a tablet/phone)
cargo xtask web --address 0.0.0.0

# Serve on a different port
cargo xtask web --port 3000
```

### 3.2 Preview the Release Web Build Locally

To test the optimized `--release` WebAssembly build locally without live reload:

```powershell
cargo xtask web --release --no-autoreload
```

Then open `http://127.0.0.1:8080`.

### 3.3 Generate the Web Production Bundle (`dist/`)

```powershell
cargo xtask web-build
```

- Runs `trunk build --release` inside `apps/papercraft-web` (configured in `apps/papercraft-web/Trunk.toml`).
- Writes the static bundle (`index.html`, `.js`, and `.wasm`) into **`dist/`** (ignored by Git).
- Ready to deploy to GitHub Pages or any static file server (see [GITHUB_PAGES.md](GITHUB_PAGES.md)).

---

## 4. Self-Hosted Sync Server (`papercraft-server` Tray App & `papercraft-server-cli`)

The sync server is optional. PaperCraft works offline without it; run either `papercraft-server` (visual tray control app) or `papercraft-server-cli` (terminal/headless CLI) when you want accounts and notebook synchronization across Desktop and Web clients.

### 4.1 System-Tray Server Control App (`papercraft-server` — Recommended for Desktop)

For everyday desktop use without terminal flags or environment variables:

```powershell
# Run in development mode
cargo run -p papercraft-server

# Generate standalone Windows release binary (target\release\papercraft-server.exe, embedded icon, no console window)
cargo build --release -p papercraft-server

# Run the compiled Windows binary (or double-click it in Windows Explorer)
.\target\release\papercraft-server.exe
```

- Runs in the OS system tray with a minimalist control window (closing the window or clicking **Hide to Tray** hides it while keeping the server running; use **Quit** in the footer or tray menu to flush `redb` and exit cleanly).
- Place `papercraft-server` (`.exe`, `.AppImage`, or macOS `.app`) in the permanent folder where you want to keep and run it before launching: settings persist in `<executable_dir>\server_control_config.json`, the default database lives in `<executable_dir>\server-data` (or any directory chosen via **Browse...**), and the **Create Desktop shortcut automatically** switch under **App Updates** keeps an OS Desktop shortcut pointed at that permanent path (and refreshes it automatically on every update).

### 4.2 Headless CLI Server (`papercraft-server-cli`) — Development Mode

```powershell
cargo run -p papercraft-server-cli -- serve --data-dir target/dev-server-data --bind 127.0.0.1:8787
```

- Stores development database files in `target/dev-server-data` (ignored by Git).
- Listens on **`http://127.0.0.1:8787`** (both the Desktop app and the Web dev server connect to this URL by default).
- Health check endpoint: open **`http://127.0.0.1:8787/api/v1/health`** in your browser.
- Press `Ctrl+C` to flush the database and stop cleanly.

### 4.3 Headless CLI Server — Release Build & Production Run

```powershell
cargo build --release -p papercraft-server-cli
```

- Generates **`target\release\papercraft-server-cli.exe`** (on Linux/macOS: `target/release/papercraft-server-cli`).

**Windows (PowerShell):**

```powershell
.\target\release\papercraft-server-cli.exe serve `
  --data-dir C:\papercraft-server `
  --bind 0.0.0.0:8787
```

**Linux / macOS (Bash):**

```bash
./target/release/papercraft-server-cli serve \
  --data-dir /var/lib/papercraft \
  --bind 0.0.0.0:8787
```

- `--bind 0.0.0.0:8787` allows connections from other devices on your LAN or encrypted mesh VPN (use `127.0.0.1:8787` if local only).
- Add `--disable-registration` after creating your accounts to block new public registrations.
- For settings mapping, environment variables, firewall rules, HTTPS reverse proxy notes, and REST API details, see [self-hosted-sync-server.md](self-hosted-sync-server.md).

### 4.4 Server Admin Commands

> [!NOTE]
> `redb` opens the database exclusively. Stop `papercraft-server-cli` (`Ctrl+C`) or click **Stop** / **Quit** in `papercraft-server` before running admin commands against the same data directory.

During development, prefix with `cargo run -p papercraft-server-cli --`; in production, call `.\papercraft-server-cli.exe` directly:

```powershell
# Inspect database counts (users, notebooks, tombstones, active sessions)
cargo run -p papercraft-server-cli -- status --data-dir target/dev-server-data

# List registered accounts
cargo run -p papercraft-server-cli -- users --data-dir target/dev-server-data

# Reset a user's password to a generated one-time recovery code (or pass --password / --temporary)
cargo run -p papercraft-server-cli -- reset-password --data-dir target/dev-server-data --username alice

# Delete a user and remove their synced notebooks, tombstones, and tokens
cargo run -p papercraft-server-cli -- delete-user --data-dir target/dev-server-data --user alice
```

---

## 5. Running Everything Together in Development Mode

To test **Desktop + Web + Sync** side by side, open **3 terminals** at the workspace root:

1. **Terminal 1 — Sync Server (Tray App or CLI):**
   ```powershell
   # Recommended GUI tray runner:
   cargo run -p papercraft-server

   # Or headless CLI server:
   cargo run -p papercraft-server-cli -- serve --data-dir target/dev-server-data --bind 127.0.0.1:8787
   ```
2. **Terminal 2 — Web App:**
   ```powershell
   cargo xtask web
   ```
   Open `http://127.0.0.1:8080` in your browser.
3. **Terminal 3 — Windows / Desktop App:**
   ```powershell
   cargo run -p papercraft
   ```

In both apps, open the Home **Profile** drawer (top-right icon), click **+ Add Account**, keep `http://127.0.0.1:8787`, create an account on one client, and sign in with the same credentials on the other.

---

## 6. Other Commands (CLI, Verification & Assets)

You do **not** need these to run or build the app normally; they are used for scripting, QA checks, and branding updates.

### 6.1 Headless CLI (`papercraft-cli`)

```powershell
# List all registered engine commands
cargo run -p papercraft-cli -- commands

# Render the built-in handwriting & stabilization showcase to PNG
cargo run -p papercraft-cli -- demo --output target/demo_showcase.png

# Execute a JSON array of {"id": "...", "params": {...}} commands and render to PNG
cargo run -p papercraft-cli -- run path/to/script.json --output target/page.png

# Build release CLI binary (target\release\papercraft-cli.exe)
cargo build --release -p papercraft-cli
```

### 6.2 Workspace Verification & CI Checks

```powershell
# Full CI check suite (layer boundaries, formatting, clippy, and all tests)
cargo xtask ci

# Check only architecture layer rules (L0 -> L6)
cargo xtask layers

# Individual checks
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# WebAssembly (wasm32) target check and lint
cargo check --target wasm32-unknown-unknown -p papercraft-web --all-targets
cargo clippy --target wasm32-unknown-unknown -p papercraft-web --all-targets --no-deps -- -D warnings
```

### 6.3 Regenerating Branding Icons

Regenerates PNG, Windows `.ico`, Apple `.icns`, and runtime `.rgba` icons from `apps/papercraft/assets/branding/logo.png`:

```powershell
cargo xtask branding
# Or on Windows:
scripts\generate-branding.cmd
```

---

## 7. Where Outputs Are Saved

| Path | What it contains |
|---|---|
| `target\debug\` | Development binaries built by `cargo run` / `cargo build` |
| `target\release\` | Optimized production binaries (`papercraft.exe`, `papercraft-server.exe`, `papercraft-server-cli.exe`, `papercraft-cli.exe` on Windows) built by `cargo build --release` |
| `target\dev-server-data\` | Suggested development database directory for `papercraft-server-cli` (Git-ignored) |
| `dist\` | Static WebAssembly production bundle generated by `cargo xtask web-build` (Git-ignored) |
| `<executable_dir>\library\` | Portable Desktop local notebooks, preferences, and sync session next to `papercraft.exe` |
| `<executable_dir>\server_control_config.json` | Persisted GUI settings for `papercraft-server` (location, network scope, port, sign-ups, limits, tray/theme options) |
| `<executable_dir>\server-data\` | Default portable database directory (`papercraft.redb` and `server_meta.json`) created by `papercraft-server` unless changed in the UI |

> [!TIP]
> **Portable folder tip:** When using downloaded or compiled release binaries of **`papercraft`** and **`papercraft-server`**, copy each executable (`.exe` / `.AppImage`) or `.app` bundle into the permanent, writable folder where you plan to keep it (rather than running from `Downloads` or read-only system paths like `C:\Program Files`). Portable data, in-app updates, and the **Create Desktop shortcut automatically** switch all target that folder and refresh the Desktop shortcut on every update.
