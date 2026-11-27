# Self-Hosted Account & Offline-First Sync Guide

PaperCraft includes a pure-Rust self-hosted account and synchronization server available in two interchangeable forms sharing the same `redb` database format and REST protocol:

1. **System-Tray Server Control App (`papercraft-server`)** — a standalone desktop executable with a minimalist control window and OS system-tray icon, designed so everyday users can choose where data is saved, configure network access, copy client URLs, manage accounts, and run the server without a terminal.
2. **Headless CLI Server (`papercraft-server-cli`)** — a lightweight terminal binary ideal for Linux servers, startup scripts, and database administration commands.

Both options pair with an offline-first sync client across the **Desktop** application (`papercraft`) and the **WebAssembly** browser application (`papercraft-web`).

> [!TIP]
> **Local-only vs. recommended setup:** You can use PaperCraft **100% locally and offline** without ever starting a server. However, the **ideal everyday setup** is to leave **`papercraft-server`** running quietly in your system tray in the background—this lets you create **multiple isolated local accounts** (e.g., personal, study, work) and keep Desktop and Web synchronized automatically. When you want to reach your notebooks away from home, enable **Local network / VPN** (`0.0.0.0:8787`) and connect over any **private encrypted mesh VPN** so your devices sync directly with your own machine without third-party cloud hosting.

> Looking for a quick overview of every Dev & Build command? See the [Command Reference](COMMANDS.md).

---

## 1. Architecture Overview

| Component | Layer | Crate / Path | Responsibility |
|---|---|---|---|
| Shared Wire Protocol | L2 | `crates/sync-protocol` | Versioned JSON DTOs (`PROTOCOL_VERSION = 1`), URL/username/password/ID validators, and deterministic SHA-256 canonical document/preference hashing |
| Client Sync Engine | L4 | `crates/sync` | Blocking `ureq` + `rustls` HTTP client (native), shared `SyncCycleDriver`, automatic `401` token refresh & retry, atomic storage for `active_session.json` and `sync_manifest.json`, and conflict-copy generation |
| Headless Session State | L5 | `crates/engine` (`src/sync_state.rs`) | Per-account local library isolation, O(1) edit tracking during pen strokes, offline delete tombstones, and engine commands (`sync.status`, `sync.setServerUrl`, `sync.signOut`) |
| Shared UI Shell | L6 | `crates/ui-egui` | Non-blocking sync coordinator (`sync_worker.rs`), Home right-hand **Profile & Settings** drawer (`profile_drawer.rs`), clean **Add Account** modal & shared account widgets (`account_modal.rs`) |
| Server Library & CLI | App | `apps/papercraft-server-cli` | Reusable `ServerRuntime` + `axum`/`tokio` HTTP router, `redb` ACID embedded database, Argon2id password hashing, bearer session management, CORS headers, rate limiting, and terminal admin subcommands |
| Server Tray Control App (GUI) | App | `apps/papercraft-server` | Standalone executable embedding `papercraft_server_cli::ServerRuntime` in-process, running in the OS system tray with a minimalist control window, folder picker, copyable URLs, Users management, and `server_control_config.json` persistence |
| Desktop Application | App | `apps/papercraft` | Opens `Session::open_desktop_library(...)` so the last signed-in account works seamlessly both online and offline |
| Web Application | App | `apps/papercraft-web` | Browser `fetch` (`RequestMode::Cors`) sync transport (`web_sync.rs`) and `localStorage` workspace persistence (`web_files.rs`) |

### Security & Clean-Room Properties
- **No crash constructs**: Non-test workspace code denies `unwrap()`, `expect()`, `panic!()`, `todo!()`, `unimplemented!()`, and `unreachable!()`.
- **No unsafe code**: `unsafe_code` is forbidden across all sync crates, `apps/papercraft-server-cli`, `apps/papercraft-server`, and `apps/papercraft-web` (the native Windows folder picker delegates through `papercraft-tablet`).
- **Credential hashing**: User passwords are hashed with Argon2id (`argon2`) using random salts (`getrandom`), and bearer access/refresh tokens are stored hashed with SHA-256 (`sha2`).
- **Rate limiting**: Authentication endpoints (`/api/v1/auth/register`, `/api/v1/auth/login`, `/api/v1/auth/refresh`) enforce per-client-IP attempt limits (`30` requests per `60` seconds).
- **Browser CORS support**: `papercraft-server-cli` sets `Access-Control-Allow-Origin: *` (`GET, POST, OPTIONS`, headers `Authorization, Content-Type`) so `papercraft-web` can sync directly from `http://127.0.0.1:8080` or hosted origins.

---

## 2. System-Tray Server App (`papercraft-server`, Recommended for Desktop Users)

If you want to run your own sync server with zero command-line flags or environment variables, download or build **`papercraft-server`**.

### Launch & Download Options

- **GitHub Releases (standalone GUI download)**:
  - **Windows**: `papercraft-server-windows-x86_64-portable.exe` (single portable executable — place it in its own folder and double-click to launch; no console window opens).
  - **Linux**: `papercraft-server-linux-x86_64.AppImage` (single portable AppImage — mark executable and launch; stores config and `server-data/` beside the `.AppImage`).
  - **macOS (Apple Silicon)**: `papercraft-server-macos-arm64.dmg` (drag `PaperCraft Server.app` into `Applications`; stores config and `server-data/` in `~/Library/Application Support/PaperCraft Server`).
- **From source (development or local release build)**:
  ```powershell
  # Run in development mode
  cargo run -p papercraft-server

  # Build standalone release executable (target\release\papercraft-server.exe on Windows)
  cargo build --release -p papercraft-server
  ```

> [!IMPORTANT]
> **Place both executables (`papercraft` and `papercraft-server`) in their permanent folder first:**
>
> Neither **PaperCraft App** nor **PaperCraft Server** uses a traditional installer. Move each `.exe` (Windows), `.AppImage` (Linux), or `.app` bundle (macOS) out of `Downloads` and into the **permanent, writable folder where you want to keep and run it** (for example `C:\Users\<you>\Apps\PaperCraft\` on Windows, `~/Applications/PaperCraft/` on Linux, or `/Applications` on macOS) before opening it:
>
> - On Windows and Linux, portable data (`library/` for the Desktop app, `server_control_config.json` and `server-data/` for the Server) is stored right next to each executable, and in-app self-updates swap the binary in that folder.
> - Both apps include a **Create Desktop shortcut automatically** switch enabled by default (**Settings -> System -> App Updates** in the Desktop app, and **App Updates** in `papercraft-server`). It creates an idiomatic Desktop shortcut (`.lnk` on Windows, `.desktop` + icon in `XDG_DESKTOP_DIR` on Linux, `.app` symlink on macOS) pointing to that permanent folder and **automatically refreshes the shortcut every time the app updates**.

### Control Window & System-Tray Behavior

- **Dedicated `Server` | `Users` switch**: Keeps server controls and configuration separate from a dedicated **Users** tab showing a live count badge (`Users (N)`), instant search/filter by name, `@username`, or user ID, UTC join dates, per-account notebook counts, copyable user IDs, per-user **Reset** (generates a random one-time recovery code shown in a prominently copyable card, requiring the user to set a new password at next sign-in) and **Delete** (with inline confirmation) actions, and **Refresh** support even while the server is stopped.
- **Auto-start & live status**: By default, opening `papercraft-server` starts the sync server immediately and shows a status badge (`Running`, `Stopped`, or `Needs attention`) alongside live counters for **Accounts**, **Notebooks**, **Active devices**, and **Database size**.
- **One-click client addresses**: Displays ready-to-paste URLs (for example `http://127.0.0.1:8787` on the same computer and `http://<LAN-IP>:8787` when **Local network / VPN** is enabled) with **Copy** buttons to paste directly into **PaperCraft -> Profile & Settings -> Sync** (or **Add Account -> Server Account**).
- **System-tray icon & menu**:
  - The tray icon shows a live status dot (**green** = running, **slate** = stopped, **red** = error).
  - **Left-click** the tray icon to open and focus the control window; **right-click** to open the tray menu (`Open Control Window`, `Hide Window`, `Start Server` / `Stop Server`, `Restart Server`, `Copy Server Address`, `Open Data Folder`, `Quit PaperCraft Server`).
  - Closing the control window (`X` or **Hide to Tray**) hides the window while keeping the sync server running in the background. Choose **Quit** (or **Quit PaperCraft Server** in the tray menu) to flush `redb` and exit cleanly.
- **Launch automatically at system startup**: Enabled by default under **App Updates & System**, registering per-user login startup with `--autostart` (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` on Windows, `~/Library/LaunchAgents/com.alyxlabs.papercraft.server.plist` on macOS, and `~/.config/autostart/papercraft-server.desktop` on Linux). By default it runs quietly in the system tray until you open the window manually; check **Open control window at system startup** if you want the window to appear at sign-in.
- **Safe setting updates**: Edits save automatically to `server_control_config.json` next to the executable. When runtime settings change while the server is running, a banner prompts you with **Apply & Restart** or **Revert**.

### Plain-English Settings Mapping (`server_control_config.json`)

Every control in `papercraft-server` maps directly to `server_control_config.json` (saved atomically next to the executable) and matches the headless `papercraft-server-cli` variables:

| Control in `papercraft-server` | `server_control_config.json` key | Default / Range | Equivalent CLI Flag / Env Variable |
|---|---|---|---|
| **Where to save your notebooks** (`Browse...` / `Open`) | `data_dir` | `<executable_dir>/server-data` | `--data-dir` / `PAPERCRAFT_DATA_DIR` |
| **Who can connect** (`This PC only` / `Local network / VPN` / `Custom IP`) & **Port** | `network_scope`, `custom_ip`, `port` | `LocalOnly` (`127.0.0.1`), port `8787` (`1..=65535`) | `--bind` / `PAPERCRAFT_BIND` |
| **Allow new account sign-ups** | `allow_registration` | `true` | Inverse of `--disable-registration` / `PAPERCRAFT_DISABLE_REGISTRATION` |
| **Max notebook upload size (MiB)** *(Advanced)* | `max_notebook_mb` | `128` (`1..=512`) | `--max-notebook-mb` / `PAPERCRAFT_MAX_NOTEBOOK_MB` |
| **Stay signed in (days)** *(Advanced)* | `refresh_ttl_days` | `30` (`1..=3650`) | `--refresh-ttl-days` / `PAPERCRAFT_REFRESH_TTL_DAYS` |
| **Background login refresh (hours)** *(Advanced)* | `access_ttl_hours` | `24` (`1..=8760`) | `--access-ttl-hours` / `PAPERCRAFT_ACCESS_TTL_HOURS` |
| **Start server automatically when this app opens** *(Advanced)* | `auto_start_server` | `true` | — (GUI launcher option) |
| **Start minimized in the system tray** *(Advanced)* | `start_in_tray` | `false` | — (GUI launcher option) |
| **Check for updates automatically on startup** *(App Updates & System)* | `auto_check_updates` | `true` | — (GUI update check) |
| **Create Desktop shortcut automatically** *(App Updates & System)* | `auto_create_desktop_shortcut` | `true` | — (GUI Desktop shortcut sync) |
| **Launch automatically at system startup** *(App Updates & System)* | `launch_at_login` | `true` | `--autostart` (OS per-user login autostart) |
| **Open control window at system startup** *(App Updates & System)* | `open_window_at_login` | `false` | — (Controls whether `--autostart` opens the window or stays hidden in tray) |
| **Light / Dark theme toggle** (header icon) | `theme` | `"Dark"` (`"Light"` / `"Dark"`) | — (GUI appearance) |

- **Environment variable import on first launch**: If `server_control_config.json` does not yet exist when `papercraft-server` first starts, any `PAPERCRAFT_*` (or legacy `NOTECRAFT_*`) environment variables already set in your environment are imported automatically as initial defaults.
- **Database interoperability**: `papercraft-server` and `papercraft-server-cli` read and write the exact same `papercraft.redb` and `server_meta.json` files. To inspect users or reset a password with `papercraft-server-cli` admin commands, click **Stop** (or **Quit**) in `papercraft-server` first so `redb` releases its file lock.

---

## 3. Building Production Binaries, Release Assets & Web Bundle

> [!NOTE]
> If you only want to run and test in **development mode**, skip directly to [Section 4](#4-running-in-development-mode) — `cargo run` and `cargo xtask web` compile automatically. Use the commands below when generating standalone release executables or the static Web bundle.

Run commands from the repository root:

| Target | Release build command | Output |
|---|---|---|
| **Server Tray App (Windows / Desktop OS)** | `cargo build --release -p papercraft-server` | `target\release\papercraft-server.exe` *(or `target/release/papercraft-server`)* |
| **Sync Server CLI (Windows / Desktop OS)** | `cargo build --release -p papercraft-server-cli` | `target\release\papercraft-server-cli.exe` *(or `target/release/papercraft-server-cli`)* |
| **Desktop / Windows App** | `cargo build --release -p papercraft` | `target\release\papercraft.exe` *(or `target/release/papercraft`)* |
| **All native binaries (App + Server Tray + Server CLI + CLI)** | `cargo build --release -p papercraft -p papercraft-server -p papercraft-server-cli -p papercraft-cli` | `target\release\*.exe` *(or `target/release/*`)* |
| **Web static bundle (WebAssembly)** | `cargo xtask web-build` | `dist\` folder (ready for GitHub Pages / static hosting; see [GITHUB_PAGES.md](GITHUB_PAGES.md)) |

*(Optional: for WebAssembly, install `rustup target add wasm32-unknown-unknown` and `cargo install --locked trunk` once before running `cargo xtask web-build`.)*

The server and client binaries are completely independent: you can run `papercraft-server` or `papercraft-server-cli` on a host PC or home server and run Desktop or Web clients elsewhere without shipping source files.

---

## 4. Running in Development Mode

You can test sync with the **Web** client, the **Desktop** client, or **both at once** using separate terminals from the repository root.

### Terminal 1 — Start the Sync Server (`papercraft-server` or `papercraft-server-cli`)

Option A (GUI Tray Control App):

```bash
cargo run -p papercraft-server
```

Option B (Headless CLI Server):

```bash
cargo run -p papercraft-server-cli -- serve --data-dir target/dev-server-data --bind 127.0.0.1:8787
```

- With Option B, `--data-dir target/dev-server-data` stores the dev database inside `target/` (ignored by Git) so production data and `git status` stay clean.
- Both default to `http://127.0.0.1:8787`. You can verify the server is running by opening `http://127.0.0.1:8787/api/v1/health` in a browser.

### Terminal 2 — Start the Web App in Development Mode (`papercraft-web`)

Install the WebAssembly target and [Trunk](https://trunkrs.dev/) once if you haven't already:

```bash
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
```

Start the hot-reloading Web dev server from the workspace root:

```bash
cargo xtask web
```

*(Equivalent to running `cd apps/papercraft-web && trunk serve`.)*

Then open **`http://127.0.0.1:8080`** in your browser.

- **Hot-reload development**: `cargo xtask web` builds the debug `wasm32-unknown-unknown` bundle, watches Rust and HTML files, rebuilds automatically on save, and reloads the browser tab.
- **Useful dev options** (forwarded directly to `trunk serve`):
  ```bash
  # Open the browser automatically once built
  cargo xtask web --open

  # Expose the Web dev server to your LAN (e.g. to test on a tablet)
  cargo xtask web --address 0.0.0.0

  # Preview the optimized release WebAssembly bundle locally without auto-reload
  cargo xtask web --release --no-autoreload
  ```
- **LAN testing tip**: If you test the Web app from another device on your local network (`cargo xtask web --address 0.0.0.0`), switch `papercraft-server` to **Local network / VPN** (or pass `--bind 0.0.0.0:8787` to `papercraft-server-cli`) and enter `http://<HOST_LAN_IP>:8787` as the server address in the Web client.

### Terminal 3 — Start the Desktop App (Optional)

Run the native desktop app alongside the server (or alongside Web to test cross-client sync):

```bash
cargo run -p papercraft
```

### Connecting & Testing Sync (Web and/or Desktop)

1. Open PaperCraft (Web at `http://127.0.0.1:8080` and/or the Desktop window).
2. Click the top-right **Profile** icon in the Home header to open the **Profile & Settings** drawer, then click **+ Add Account** (or **Connect Server**).
3. Keep `http://127.0.0.1:8787` (or paste the copied URL from `papercraft-server`) in **Self-Hosted Server** and click **Test** to verify `Connected`.
4. Switch to **Create Account**, enter an email, optional `@username` (`3–32` characters), and password (`8–128` characters), and submit.
5. On a second client (for example, sign into Web with the account you created on Desktop, or vice versa), choose **Sign In** with the same URL and credentials to see notebooks sync across both.
6. **Verify offline resilience**: stop the server (**Stop** in `papercraft-server`, or `Ctrl+C` in Terminal 1), create or edit notebooks in Desktop or Web (even close and reopen Desktop or refresh the Web tab), then start the server again and click **Sync Now** (or wait for auto-sync).

> [!NOTE]
> **HTTPS / Mixed-Content rule for browsers:** When running Web locally at `http://127.0.0.1:8080`, `http://127.0.0.1:8787` works directly. When using the hosted `https://` web client ([web.papercraft.ink](https://web.papercraft.ink/)) or your own `https://` GitHub Pages deployment, browsers block requests to remote `http://` addresses—place `papercraft-server-cli` or `papercraft-server` behind an `https://` TLS reverse proxy for remote HTTPS web access.

---

## 5. Running Headless CLI in Production (Local Host & Mesh VPN)

### Starting the Server

To accept connections from both the local machine (`127.0.0.1`) and peer devices over a LAN or encrypted mesh VPN, bind to `0.0.0.0:8787` (or your host's specific VPN interface IP).

**Windows (PowerShell):**

```powershell
.\papercraft-server-cli.exe serve `
  --data-dir C:\papercraft-server `
  --bind 0.0.0.0:8787
```

**Linux / macOS (Bash):**

```bash
./papercraft-server-cli serve \
  --data-dir /var/lib/papercraft \
  --bind 0.0.0.0:8787
```

Once your personal accounts are registered, lock new public sign-ups with `--disable-registration`:

**Windows (PowerShell):**

```powershell
.\papercraft-server-cli.exe serve `
  --data-dir C:\papercraft-server `
  --bind 0.0.0.0:8787 `
  --disable-registration
```

**Linux / macOS (Bash):**

```bash
./papercraft-server-cli serve \
  --data-dir /var/lib/papercraft \
  --bind 0.0.0.0:8787 \
  --disable-registration
```

### Configuring via Environment Variables

Useful for background services, startup scripts, or `systemd` units:

**Windows (PowerShell):**

```powershell
$env:PAPERCRAFT_DATA_DIR = "C:\papercraft-server"
$env:PAPERCRAFT_BIND = "0.0.0.0:8787"
$env:PAPERCRAFT_DISABLE_REGISTRATION = "true"
$env:RUST_LOG = "info"
.\papercraft-server-cli.exe
```

**Linux / macOS (Bash):**

```bash
PAPERCRAFT_DATA_DIR=/var/lib/papercraft \
PAPERCRAFT_BIND=0.0.0.0:8787 \
PAPERCRAFT_DISABLE_REGISTRATION=true \
RUST_LOG=info \
./papercraft-server-cli
```

### Network, Firewall & Transport Security
- **Same machine**: connect clients to `http://127.0.0.1:8787`.
- **Across devices over LAN or mesh VPN**: connect Desktop (or HTTP Web) clients to `http://<SERVER_VPN_IP>:8787`, or `https://<SERVER_DOMAIN>` when placed behind a TLS reverse proxy.
- **Windows Firewall**: if peer devices over the VPN/LAN cannot reach port `8787`, allow inbound TCP `8787`:
  ```powershell
  New-NetFirewallRule -DisplayName "PaperCraft Server (VPN)" -Direction Inbound -Protocol TCP -LocalPort 8787 -Action Allow
  ```
- **Transport security**: a private encrypted mesh VPN already encrypts traffic between nodes; if exposing over an untrusted network or connecting from an `https://`-hosted Web client, terminate TLS (`https://`) with a reverse proxy and keep the sync server bound to `127.0.0.1:8787`.

---

## 6. Server CLI & Admin Reference

`serve` is the default subcommand if no subcommand is given.

### Server Options (`serve` or top-level)

| Flag | Environment Variable | Default | Range / Notes | Description |
|---|---|---|---|---|
| `--data-dir <PATH>` | `PAPERCRAFT_DATA_DIR` | `papercraft-server-data` | Path | Directory holding `papercraft.redb` and `server_meta.json` |
| `--bind <ADDR>` | `PAPERCRAFT_BIND` | `127.0.0.1:8787` | `IP:PORT` | TCP listen address (`0.0.0.0:8787` for LAN/VPN) |
| `--disable-registration` | `PAPERCRAFT_DISABLE_REGISTRATION` | `false` | Boolean | Rejects new `/api/v1/auth/register` requests (`403`) |
| `--access-ttl-hours <N>` | `PAPERCRAFT_ACCESS_TTL_HOURS` | `24` | `1..=8760` | Bearer access token lifetime in hours |
| `--refresh-ttl-days <N>` | `PAPERCRAFT_REFRESH_TTL_DAYS` | `30` | `1..=3650` | Refresh token lifetime in days |
| `--max-notebook-mb <N>` | `PAPERCRAFT_MAX_NOTEBOOK_MB` | `128` | `1..=512` | Max compact `.pcraft` notebook upload size (MiB) |

### Administrative Subcommands

> [!NOTE]
> Stop any running `papercraft-server-cli` (`Ctrl+C`) or click **Stop** / **Quit** in `papercraft-server` before running admin commands on the same `--data-dir`.

Pass `--data-dir` **after** the subcommand name:

**Windows (PowerShell, compiled binary):**

```powershell
# Inspect database counts (users, active notebooks, tombstones, active sessions)
.\papercraft-server-cli.exe status --data-dir C:\papercraft-server

# List registered accounts, display names, and notebook counts
.\papercraft-server-cli.exe users --data-dir C:\papercraft-server

# Reset a user's password to a generated one-time recovery code (omit --password)
.\papercraft-server-cli.exe reset-password --data-dir C:\papercraft-server --username alice

# Or set an explicit password immediately (add --temporary to require a change on next sign-in)
.\papercraft-server-cli.exe reset-password --data-dir C:\papercraft-server --username alice --password "new-strong-password" --temporary

# Delete a user and remove their synced notebooks, tombstones, preferences, and tokens
.\papercraft-server-cli.exe delete-user --data-dir C:\papercraft-server --user alice
```

**Linux / macOS (Bash, compiled binary):**

```bash
./papercraft-server-cli status --data-dir /var/lib/papercraft
./papercraft-server-cli users --data-dir /var/lib/papercraft
./papercraft-server-cli reset-password --data-dir /var/lib/papercraft --username alice
./papercraft-server-cli reset-password --data-dir /var/lib/papercraft --username alice --password "new-strong-password" --temporary
./papercraft-server-cli delete-user --data-dir /var/lib/papercraft --user alice
```

**Development mode (`cargo run` from workspace root):**

```bash
cargo run -p papercraft-server-cli -- status --data-dir target/dev-server-data
cargo run -p papercraft-server-cli -- users --data-dir target/dev-server-data
cargo run -p papercraft-server-cli -- reset-password --data-dir target/dev-server-data --username alice
cargo run -p papercraft-server-cli -- delete-user --data-dir target/dev-server-data --user alice
```

---

## 7. Offline-First Storage, Account Modes & Sync Behavior

### Offline-First Server Accounts

- **First sign-in / registration**: Requires an online server connection to authenticate and download the account's notebooks, folders, and studio preferences.
- **Persistent per-account local cache**: Once signed in on a device, notebooks, folders, preferences, and pending upload manifests are persisted locally per server account (`<executable_dir>/library/accounts/<user_id>/` on Desktop and `papercraft_workspace_v1` `localStorage` on Web).
- **Resilient offline work & sync queue**: If connectivity or server availability drops during a session, PaperCraft never signs you out or locks your notes. Edits continue smoothly against the cached copy (`Offline — changes saved locally`), enter the pending sync queue, and upload automatically upon reconnect.
- **Switching accounts offline**: Local accounts are always accessible. Previously signed-in server accounts with a cache on the device show **"Offline mode — will sync on reconnect"** and can be opened anytime; new server sign-ins without a local cache are blocked until online.
- **Forget Account (Privacy Wipe)**: Clicking **Forget Account** on a server account warns if unsynced local changes exist, then removes the shortcut and deletes **only** that account's local cache on the device while leaving all server data intact.
- **Safe conflict resolution**: Offline changes made on different pages of the same notebook are combined automatically via 3-way page-level merge. If two devices modify the same page offline, PaperCraft keeps the authoritative server copy and preserves the local edits as `"<Title> (Conflict Copy)"`.

### Local Accounts (Device-Only)

- **Desktop local libraries**: Stored per local profile under `<executable_dir>/library/profiles/<profile_id>/`.
- **Web local cache**: Local profiles are stored in browser `localStorage` under `papercraft_workspace_v1`.
- **Convert local to server ("Sync")**: When a configured server is online, any local account card in **Profile -> Accounts** exposes a **Sync** button that prompts for email and password, uploads all local notebooks, folders, and studio preferences to a new server account, retains all data on the device as the active Offline-First cache, and begins background sync.

### How Background Server Sync Works
1. **Non-blocking triggers**: Sync runs asynchronously (dedicated worker thread `papercraft-sync` on Desktop; async browser `fetch` via `wasm-bindgen-futures` on Web) with SHA-256 content & per-page hashing offloaded from the UI thread and an adaptive trailing debounce (1.5 s normally, scaling to 3.5 s at >= 4 MiB or 6.0 s at >= 32 MiB of pending binary attachments), plus every 12 seconds when idle or manually via **Sync Now**.
2. **Active-stroke protection**: Pulled updates are held until `live_stroke` and `live_eraser` are idle so active handwriting is never interrupted.
3. **Tombstone deletions**: Deleting a server notebook records a deletion tombstone synced directly to the server (unless concurrent offline edits exist remotely).

---

## 8. Server Database & REST API Reference

### Storage Files (`--data-dir` / `server-data`)
- `papercraft.redb`: single-file `redb` ACID database containing tables `users`, `users_by_name`, `access_tokens`, `refresh_tokens`, `notebooks`, `notebook_blobs`, `user_prefs`, and `user_cursors` (legacy `notecraft.redb` files are still opened automatically if present).
- `server_meta.json`: persistent `server_id`, creation timestamp, and `protocol_version`.
- `server_control_config.json` *(next to `papercraft-server` executable)*: persisted GUI settings for the system-tray control app.

**Backup procedure**: stop the server (**Stop** or **Quit** in `papercraft-server`, or `Ctrl+C` in `papercraft-server-cli`) so all writes are finalized, then copy `papercraft.redb` and `server_meta.json` (or the whole data directory).

### HTTP Endpoints (`/api/v1`)

| Method & Path | Auth | Request / Query | Response | Notes |
|---|---|---|---|---|
| `GET /api/v1/health` | Public | — | `200 ServerInfoResponse` | Reports `service`, `version`, `protocol_version`, `registration_open`, `storage_engine`, and `max_notebook_bytes` |
| `POST /api/v1/auth/register` | Public | `RegisterRequest` | `201 AuthResponse` | Respects `--disable-registration` (`403`) and case-insensitive username/email uniqueness (`409`) |
| `POST /api/v1/auth/login` | Public | `LoginRequest` | `200 AuthResponse` | Verifies Argon2id hash and issues fresh access + refresh tokens (returns `403 PASSWORD_RESET_REQUIRED` when a one-time recovery code must be completed) |
| `POST /api/v1/auth/complete-password-reset` | Public | `CompletePasswordResetRequest` | `200 AuthResponse` | Verifies the generated one-time recovery code, saves the new password, clears `must_change_password`, and signs the user in |
| `POST /api/v1/auth/refresh` | Public | `RefreshRequest` | `200 AuthResponse` | Single-use refresh token rotation |
| `POST /api/v1/auth/logout` | Optional Bearer | `LogoutRequest` | `204 No Content` | Revokes presented access and/or refresh tokens |
| `GET /api/v1/auth/me` | Bearer | — | `200 UserProfile` | Returns current authenticated account profile |
| `POST /api/v1/auth/update-profile` | Bearer | `UpdateProfileRequest` | `200 UserProfile` | Updates `display_name` and/or `@username` |
| `POST /api/v1/auth/change-password` | Bearer | `ChangePasswordRequest` | `200 UserProfile` | Verifies current password and updates the account password hash |
| `GET /api/v1/sync/pull` | Bearer | `?since=<u64>` | `200 SyncPullResponse` | Returns `server_cursor`, changed notebooks with cursor `> since`, tombstones (when `since > 0`), and `studio_prefs` |
| `POST /api/v1/sync/push` | Bearer | `SyncPushRequest` | `200 SyncPushResponse` | Up to `64` notebook changes per request (`MAX_PUSH_BATCH_SIZE`), returning `accepted`, `conflicts`, `server_cursor`, and `prefs_version` |

---

## 9. Workspace Verification Commands

```bash
# Run the full CI check suite (layers, formatting, clippy, and all workspace tests)
cargo xtask ci

# Individual workspace checks
cargo xtask layers
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# WebAssembly (wasm32) target checks
cargo check --target wasm32-unknown-unknown -p papercraft-web --all-targets
cargo clippy --target wasm32-unknown-unknown -p papercraft-web --all-targets --no-deps -- -D warnings
```
