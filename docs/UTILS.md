# PaperCraft Utility & Testing Commands

Quick-reference commands for running, testing, resetting, and verifying PaperCraft locally without disturbing an existing production installation.

---

## Core Isolation Rules (Safe Alongside a Production Install)

- **Portable storage beside raw debug binaries**: Debug runs (`cargo run`) store data inside `target/debug/library` (Desktop) and `target/debug/server-data` + `target/debug/server_control_config.json` (Server).
- **OS shortcuts and login autostart stay untouched**: Debug builds skip automatic startup synchronization of Desktop shortcuts and OS login autostart (`#[cfg(not(debug_assertions))]`). Avoid manually toggling **Desktop Shortcut** or **Launch at Login** inside test instances.
- **Alternate server port (`8788`)**: Running the test server on `127.0.0.1:8788` prevents conflicts with, or writes to, a live sync server listening on the default port `8787`.
- **Client URL for tests**: Always enter **`http://127.0.0.1:8788`** as the Server URL in test Desktop and Web sessions.

*(If `target/debug/server_control_config.json` already exists from a previous run using port `8787`, change the port to `8788` in the server control window or delete that config file before starting.)*

---

## Windows (PowerShell)

Run each app in a separate PowerShell terminal from the repository root:

### Daily Isolated Testing (`127.0.0.1:8788`)

- **Test Sync Server (Tray UI):**
  ```powershell
  $env:PAPERCRAFT_BIND="127.0.0.1:8788"; cargo run -p papercraft-server
  ```
- **Test Sync Server (Headless CLI):**
  ```powershell
  cargo run -p papercraft-server-cli -- serve --bind 127.0.0.1:8788 --data-dir target\debug\server-data
  ```
- **Test Desktop App (Debug):**
  ```powershell
  cargo run -p papercraft
  ```
- **Test Web App (WebAssembly — open `http://127.0.0.1:8080`):**
  ```powershell
  cargo xtask web
  ```

### Reset Local Test Data (Close Test Apps First)

```powershell
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue target\debug\library, target\debug\server-data, target\debug\server_control_config.json
```

### Standard Run & Release Build

- **Default Sync Server CLI (`127.0.0.1:8787`):**
  ```powershell
  cargo run -p papercraft-server-cli -- serve --bind 127.0.0.1:8787
  ```
- **Build & Launch Release Executables (`target\release\*.exe`):**
  ```powershell
  cargo build --release -p papercraft -p papercraft-server
  Start-Process .\target\release\papercraft-server.exe
  Start-Process .\target\release\papercraft.exe
  ```
  *(Note: Release binaries sync Windows Desktop shortcuts and login autostart on startup when enabled.)*

---

## Linux (Bash / zsh)

Run each app in a separate shell from the repository root:

### Daily Isolated Testing (`127.0.0.1:8788`)

- **Test Sync Server (Tray UI):**
  ```bash
  PAPERCRAFT_BIND=127.0.0.1:8788 cargo run -p papercraft-server
  ```
- **Test Sync Server (Headless CLI):**
  ```bash
  cargo run -p papercraft-server-cli -- serve --bind 127.0.0.1:8788 --data-dir target/debug/server-data
  ```
- **Test Desktop App (Debug):**
  ```bash
  cargo run -p papercraft
  ```
- **Test Web App (WebAssembly — open `http://127.0.0.1:8080`):**
  ```bash
  cargo xtask web
  ```

### Reset Local Test Data (Close Test Apps First)

```bash
rm -rf target/debug/library target/debug/server-data target/debug/server_control_config.json
```

### Standard Run & Release Build

- **Default Sync Server CLI (`127.0.0.1:8787`):**
  ```bash
  cargo run -p papercraft-server-cli -- serve --bind 127.0.0.1:8787
  ```
- **Build & Launch Release Binaries (`target/release/`):**
  ```bash
  cargo build --release -p papercraft -p papercraft-server
  ./target/release/papercraft-server &
  ./target/release/papercraft &
  ```
  *(Note: Standalone binaries store data beside the binary; AppImage packages store `library/` and `server-data/` beside the `.AppImage` file. Release binaries sync `.desktop` shortcuts and autostart entries on startup when enabled.)*

---

## macOS (zsh / Bash)

Run each app in a separate terminal from the repository root:

### Daily Isolated Testing (`127.0.0.1:8788`)

- **Test Sync Server (Menu Bar / Tray UI):**
  ```zsh
  PAPERCRAFT_BIND=127.0.0.1:8788 cargo run -p papercraft-server
  ```
- **Test Sync Server (Headless CLI):**
  ```zsh
  cargo run -p papercraft-server-cli -- serve --bind 127.0.0.1:8788 --data-dir target/debug/server-data
  ```
- **Test Desktop App (Debug):**
  ```zsh
  cargo run -p papercraft
  ```
- **Test Web App (WebAssembly — open `http://127.0.0.1:8080`):**
  ```zsh
  cargo xtask web
  ```

### Reset Local Test Data (Close Test Apps First)

```zsh
rm -rf target/debug/library target/debug/server-data target/debug/server_control_config.json
```

### Standard Run & Release Build

- **Default Sync Server CLI (`127.0.0.1:8787`):**
  ```zsh
  cargo run -p papercraft-server-cli -- serve --bind 127.0.0.1:8787
  ```
- **Build & Launch Release Binaries (`target/release/`):**
  ```zsh
  cargo build --release -p papercraft -p papercraft-server
  ./target/release/papercraft-server &
  ./target/release/papercraft &
  ```
  *(Note: Raw Cargo binaries in `target/debug/` or `target/release/` store data beside the binary. Packaged `.app` bundles instead store data under `~/Library/Application Support/PaperCraft/library` and `~/Library/Application Support/PaperCraft Server/`, and release builds sync Desktop shortcuts and LaunchAgents when enabled.)*

---

## Shared Cross-Platform Tools & Checks

### WebAssembly Setup & Release Bundle (`papercraft-web`)

```bash
# First-time prerequisites:
rustup target add wasm32-unknown-unknown
cargo install --locked trunk

# Production WebAssembly bundle (outputs to apps/papercraft-web/dist/):
cargo xtask web-build
```

### Headless Engine CLI

```bash
cargo run -p papercraft-cli -- commands
```

### Local Auto-Update Testing

Start a local update server serving local binaries and `latest.json` at `http://127.0.0.1:8799/latest.json`:

```bash
cargo xtask update-dev-server --version 0.2.0
```

Set `PAPERCRAFT_UPDATE_MANIFEST_URL` and `PAPERCRAFT_UPDATE_CURRENT_VERSION` on the client or server process being tested to validate updates safely offline/locally.

### Scoped Checks, Tests & Workspace CI

```bash
# Scoped checks & unit tests:
cargo check -p papercraft-ui-egui
cargo check -p papercraft-server
cargo test -p papercraft-engine --lib
cargo test -p papercraft-ui-egui
cargo test -p papercraft-server-cli

# Workspace architecture layers & full CI:
cargo xtask layers
cargo xtask ci

# WebAssembly check & clippy:
cargo check --target wasm32-unknown-unknown -p papercraft-web --all-targets
cargo clippy --target wasm32-unknown-unknown -p papercraft-web --all-targets --no-deps -- -D warnings
```
