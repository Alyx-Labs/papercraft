# Installers, Packaging & Update Architecture — PaperCraft

Planning and architectural reference for distributing **PaperCraft** (`papercraft` desktop app and optional `papercraft-server` tray app) as native installers alongside the existing portable builds.

> [!NOTE]
> **Alpha status:** PaperCraft currently ships **portable-only** artifacts so the workspace stays lightweight during concept validation. This document captures the design options, OS storage rules, and update trade-offs to guide future installer and packaging work without breaking existing portable users.

---

## 1. Current Baseline (What Exists Today)

PaperCraft is built in **pure Rust** (`eframe` / `egui` / `wgpu` on Desktop, plus WebAssembly for Web) and publishes releases through [`.github/workflows/release.yml`](../.github/workflows/release.yml):

| Platform | Current release artifacts | Current data location | Current self-update behavior |
|---|---|---|---|
| **Windows (`x86_64`)** | Unsigned standalone portable `.exe` (`papercraft-windows-x86_64-portable.exe`, `papercraft-server-windows-x86_64-portable.exe`) | `library/` and `server-data/` beside the `.exe` | Downloads `.exe` from `latest.json`, verifies SHA-256, swaps in place, relaunches |
| **Linux (`x86_64`)** | `.AppImage` (`papercraft`, `papercraft-server`) | `library/` and `server-data/` beside the `.AppImage` (or exe) | Downloads `.AppImage`, verifies SHA-256, swaps file in place, relaunches |
| **macOS (`arm64`, experimental)** | Ad-hoc signed `.dmg` and `.tar.gz` | `~/Library/Application Support/PaperCraft/` and `PaperCraft Server/` | Replaces `.app` bundle via `.tar.gz` when running from a writable `.app` |
| **Web (`wasm32`)** | `papercraft-web.zip` (`trunk build --release`) | Browser `localStorage` (`papercraft_workspace_v1`) | Static web deployment |

Relevant implementation files:
- Library path resolution: [`crates/engine/src/library.rs`](../crates/engine/src/library.rs) (`default_library_dir`)
- Server control config & data path resolution: [`apps/papercraft-server/src/settings.rs`](../apps/papercraft-server/src/settings.rs) (`default_config_path`, `default_data_dir_string`)
- Update check, capability probe & swap logic: [`crates/updater/src/client.rs`](../crates/updater/src/client.rs), [`crates/updater/src/install.rs`](../crates/updater/src/install.rs), [`crates/updater/src/manifest.rs`](../crates/updater/src/manifest.rs)
- Update dialog UI: [`crates/ui-egui/src/update_modal.rs`](../crates/ui-egui/src/update_modal.rs)

---

## 2. Core Prerequisites Before Shipping Installers

Before adding native OS installers (`.msi`, `.deb`, `.rpm`, notarized `.dmg`), four structural foundations must be in place.

### 2.1 Single Binary, Dual Runtime Mode (`Portable` vs `Installed`)

PaperCraft does **not** need separate compiled binaries for portable and installed builds. One binary can detect its mode at startup using an explicit runtime rule:

1. **Explicit environment override** (e.g., `PAPERCRAFT_LIBRARY_DIR`), if set.
2. **Portable mode** when **either**:
   - A marker file (`portable.txt`) sits next to the executable (shipped inside future `-portable.zip` archives), **or**
   - A legacy `library/` folder (or `server_control_config.json` / `server-data/` for `papercraft-server`) already exists beside the executable and that folder is writable *(preserves 100% compatibility for existing Alpha portable users who do not have `portable.txt`)*, **or**
   - Running from a Linux `$APPIMAGE` (preserving sibling storage beside the `.AppImage`, or opt-in migration).
3. **Installed (standard OS) mode** otherwise: store user notebooks, logs, and config in OS-standard per-user directories, never inside the binary installation folder.

#### Recommended Directory Separation (Windows Example)

| Mode | Executables location | User data (`library/`, `studio_prefs.json`, logs, `server-data/`) |
|---|---|---|
| **Portable (`-portable.zip`)** | Any user-chosen folder (e.g., `D:\Apps\PaperCraft\`) with `portable.txt` | `<exe_dir>\library\` (self-contained next to `papercraft.exe`) |
| **Installed (`per-user` default)** | `C:\Users\<User>\AppData\Local\Programs\PaperCraft\` (no admin UAC prompt) | `C:\Users\<User>\AppData\Local\PaperCraft\` (`%LOCALAPPDATA%\PaperCraft\`) |
| **Installed (`per-machine` optional)** | `C:\Program Files\PaperCraft\` (read-only at runtime; requires UAC to install) | `C:\Users\<User>\AppData\Local\PaperCraft\` (`%LOCALAPPDATA%\PaperCraft\`) |

> [!IMPORTANT]
> **Why `%LOCALAPPDATA%` over `%APPDATA%` (`Roaming`) for `library/`:** Notebooks, imported PDFs, and embedded raster images can grow large. Storing `library/` and `server-data/` under `%LOCALAPPDATA%\PaperCraft\` avoids saturating Windows roaming profile quotas while keeping user data safe across uninstalls and upgrades. On Linux, use `$XDG_DATA_HOME/papercraft` (`~/.local/share/papercraft`) and `$XDG_CONFIG_HOME/papercraft`; on macOS, keep `~/Library/Application Support/PaperCraft/`.

### 2.2 Static C Runtime on Windows (`+crt-static`)

Building Windows release targets with `RUSTFLAGS="-C target-feature=+crt-static"` statically links the C runtime into `papercraft.exe` and `papercraft-server.exe`. Neither the installer nor the portable archive will require users to pre-install the Microsoft Visual C++ Redistributable.

### 2.3 File Associations & Startup Arguments (`.pcraft`, `.pcraftpkg`)

Installers enable OS file associations (double-clicking a `.pcraft` notebook or `.pcraftpkg` backup package to launch PaperCraft). To support that workflow end-to-end:
- [`apps/papercraft/src/main.rs`](../apps/papercraft/src/main.rs) needs to parse startup file path arguments via `std::env::args_os()` and open/import the target `.pcraft` or `.pcraftpkg` file cleanly after initializing the session.
- Installer manifests (`.wxs`, `.desktop` + MIME XML, macOS `Info.plist`) then register `.pcraft` / `.pcraftpkg` handlers pointing to the executable.

### 2.4 Dual-App Packaging (`papercraft` + `papercraft-server`)

Because PaperCraft works offline standalone and `papercraft-server` is optional:
- **Recommended:** Package `papercraft` as the primary required component and include `papercraft-server` as an **optional installer feature** (checked or unchecked by user choice, or shipped as a companion installer/package).

---

## 3. Packaging & Installer Options by Platform

### 3.1 Windows

| Option | Output | Pros | Cons | Verdict |
|---|---|---|---|---|
| **WiX Toolset v5 (`.msi`)** | `.msi` | Native Windows Installer database; clean `UpgradeCode` upgrades; supports `Scope="perUserOrMachine"` (defaulting to per-user with no UAC); enterprise- & `winget`-friendly; easy CLI install via `dotnet tool` in GitHub Actions | XML authoring (`.wxs`) has a modest initial learning curve | **Recommended long-term standard** |
| **Inno Setup / NSIS (`.exe`)** | `Setup.exe` | Very fast to author (~50-line script); pre-installed on GitHub `windows-latest` runners; familiar wizard UX; supports per-user install | Not an `.msi` database; weaker silent-upgrade / enterprise tracking than WiX | Good lightweight Alpha fallback if an `.exe` wizard is preferred over `.msi` |
| **Portable Archive (`.zip`)** | `-portable.zip` + `portable.txt` | Zero installation, USB-friendly, keeps 1-click in-place self-updates | No Start Menu entry, no Add/Remove Programs entry, no file associations | **Keep permanently alongside the installer** |
| **Web-wrapper bundlers** | Heavy `.msi`/`.exe` | Built-in webview bundler scripts | Unnecessary dependency overhead; PaperCraft is pure Rust (`eframe`/`wgpu`) with no HTML/JS desktop frontend | **Not applicable** |

**Recommended Windows release set:**
1. `papercraft-<version>-windows-<arch>.msi` (WiX v5, `Scope="perUserOrMachine"`, per-user default under `%LocalAppData%\Programs\PaperCraft\`, Start Menu shortcut, optional `papercraft-server` feature, `.pcraft`/`.pcraftpkg` associations, custom wizard bitmaps generated from `papercraft-256.png`).
2. `papercraft-<version>-windows-<arch>-portable.zip` (self-contained `.exe` + `portable.txt` + license notices).
3. Optional `-symbols.zip` (`.pdb` files) for crash profiling.
4. Both `x64` (`x86_64-pc-windows-msvc`) and `arm64` (`aarch64-pc-windows-msvc`) targets once CI runners are enabled.

### 3.2 Linux

| Format | Tooling | Pros | Cons | Verdict |
|---|---|---|---|---|
| **`.AppImage` (+ `.zsync`)** | Pinned `appimagetool` + `type2-runtime` + `zsyncmake` | Runs across distros with one file; already implemented; supports in-place self-update and `.zsync` delta updates | Needs a custom `AppRun` script to install `.desktop` + icons into `$XDG_DATA_HOME` so Wayland resolves the `app_id` icon instead of a generic icon | **Keep and harden** (pin runtime + add Wayland `AppRun`) |
| **`.deb` & `.rpm`** | **`nFPM`** (`nfpm.yaml`) | One lightweight static binary builds both Debian/Ubuntu and Fedora/openSUSE/RHEL packages from a single FHS staging tree; clean system menu & MIME integration | Must explicitly declare `dlopen`-loaded runtime libs (`libxkbcommon0`, `libxkbcommon-x11-0`, `libwayland-client0`, `libx11-6`, `libxcb1`, `libxcursor1`, `libxi6`, `libvulkan1 \| libegl1`) because ELF `NEEDED` scanning misses them | **Recommended next addition** |
| **`.pkg.tar.zst`** | `makepkg` / `PKGBUILD` | Native Arch Linux package from the staged tarball | Extra distro-specific check | Optional after `.deb`/`.rpm` |
| **`.flatpak`** | `flatpak-builder` | Sandboxed distribution & future Flathub store presence | Requires XDG desktop portals verification for file dialogs, stylus/tablet access, and external server sync | **Recommended for Beta/1.0**, not urgent for Alpha |

> [!TIP]
> Build Linux release artifacts on an older LTS base (such as `ubuntu-22.04`, `glibc 2.35`) so binaries run cleanly on both older and newer distributions.

### 3.3 macOS

Current `.github/workflows/release.yml` produces `aarch64` `.dmg` and `.tar.gz` bundles with ad-hoc `codesign --sign -`. To make macOS distribution friction-free outside local developer testing:
1. **Universal binary (`aarch64` + `x86_64`)** combined with `lipo` (supporting Apple Silicon and Intel Macs in one `.app`).
2. **Reliable CI `.dmg` creation:** replace `hdiutil create -srcfolder` (which can flake under CI runners) with `hdiutil makehybrid` followed by `hdiutil convert -format UDZO`.
3. **Developer ID signing & notarization:** sign nested executables inside-out with `codesign --options runtime` + `entitlements.plist` (avoid `--deep` on the final seal), notarize both `.app` and `.dmg` via `xcrun notarytool submit --wait`, and staple tickets via `xcrun stapler staple`. Without Apple Developer ID signing and notarization, Gatekeeper blocks downloaded builds on other Macs.

---

## 4. Auto-Update Strategy Across Installed and Portable Builds

### Why Pure Rust Differs from Two-Layer Web-Wrapper Apps

Some desktop apps built on web-browser runtimes use a **two-layer version model**:
- An **inner script/UI bundle** downloaded into user `AppData` that updates automatically without admin rights or touching the main executable.
- A **native runtime installer** updated rarely and manually by re-running the installer from the website.

Because **PaperCraft compiles its entire engine and UI into a single lightweight native binary** (only a few megabytes, with no separate script bundle), every update updates the executable itself. That gives PaperCraft four clear update options when installers are introduced:

| Strategy | How it works | Pros | Trade-offs | Fit for PaperCraft |
|---|---|---|---|---|
| **1. In-place binary / bundle swap** *(current behavior)* | Downloads verified binary/AppImage/`.app.tar.gz`, swaps on disk, relaunches | Seamless 1-click update; already implemented in `papercraft-updater` | Fails on read-only `/usr/bin` or `Program Files`; on writable per-user `.msi` installs, replacing only the `.exe` leaves Windows Installer version metadata stale | **Keep for Portable `.exe`/`.zip`, `.AppImage`, and macOS `.app`** |
| **2. Notify + Guided Manual Installer** | App checks `latest.json`, displays release notes + notice banner, opens release/download page | Zero UAC, packaging-DB, or Windows SmartScreen friction; works identically across `.msi`, `.deb`, `.rpm`; **UI flow already exists** in `update_modal.rs` (`InstallCapability::ReadOnly` / `UnsupportedLayout`) | User clicks through the browser download + installer | **Best immediate Alpha/unsigned behavior for `.msi`, `.deb`, `.rpm`** |
| **3. In-App Installer Execution (`msiexec`)** | App detects MSI installation, downloads & verifies `.msi` via SHA-256, exits, runs `msiexec /i <pkg>.msi /passive`, and relaunches | 1-click in-app update while keeping Windows "Installed Apps" version, shortcuts, and file associations in sync | Without Authenticode code signing, Windows SmartScreen may warn on the newly downloaded `.msi`; per-machine (`Program Files`) updates trigger a UAC prompt | **Best long-term behavior for Windows `.msi` once signed** |
| **4. OS Package Managers / Stores** | `winget`, Homebrew Cask, Flathub, distro repos | OS handles verification, elevation, and background updates | Requires automated PRs/manifests per release | **Complement to Strategy 2/3** |

### Crucial Detail for Per-User Windows Installers

In [`crates/updater/src/install.rs`](../crates/updater/src/install.rs), `probe_install_capability_for` currently only probes whether the executable's parent directory is writable (`check_dir_writable`).

If a future `.msi` installs per-user into `%LocalAppData%\Programs\PaperCraft\`, that folder **is** writable. Without detecting MSI registration (or absence of `portable.txt`), the current updater would overwrite `papercraft.exe` in place while leaving Windows Installer metadata stuck on the old version.

**Required updater update when adding `.msi`:**
1. Extend `latest.json` (`AssetFormat`) to publish both portable and installer assets (e.g., `windows-x86_64` portable `.exe`/`.zip` + `windows-x86_64-msi`).
2. Update `probe_install_capability` to detect whether the running binary was installed by MSI (for example via MSI install registry metadata or install layout).
3. Route MSI installs either to **Strategy 2** (`UnsupportedLayout` -> notice + `"Open Release Page"` / direct `.msi` link while unsigned) or **Strategy 3** (verified `.msi` download -> `msiexec /i ... /passive` + relaunch once signed).

---

## 5. Phased Implementation Roadmap

1. **Phase 0 — Storage & Build Hygiene (Pre-Installer):**
   - Implement `portable.txt` + legacy sibling-folder detection alongside OS-standard data paths (`%LOCALAPPDATA%\PaperCraft` on Windows, `$XDG_DATA_HOME/papercraft` on Linux, existing `~/Library/Application Support/PaperCraft` on macOS) in [`crates/engine/src/library.rs`](../crates/engine/src/library.rs) and [`apps/papercraft-server/src/settings.rs`](../apps/papercraft-server/src/settings.rs).
   - Enable `+crt-static` for Windows release builds in `.github/workflows/release.yml`.
   - Wire `std::env::args_os()` startup file opening for `.pcraft` / `.pcraftpkg` in [`apps/papercraft/src/main.rs`](../apps/papercraft/src/main.rs).
2. **Phase 1 — First Native Installers (Alpha / Beta):**
   - **Windows:** Add `packaging/windows/papercraft.wxs` (WiX v5, `perUserOrMachine` with per-user default, optional `papercraft-server` feature) + `-portable.zip` packaging.
   - **Linux:** Add `packaging/linux/nfpm.yaml` (`.deb` + `.rpm`), pin `appimagetool` + `type2-runtime`, and add a Wayland-aware `AppRun` script.
   - **macOS:** Switch `.dmg` creation to `hdiutil makehybrid` + `convert -format UDZO` and add universal (`aarch64` + `x86_64`) support.
3. **Phase 2 — Installer-Aware Updater:**
   - Teach `papercraft-updater` and `latest.json` to distinguish **Portable** vs **MSI / System Package** installs, preventing silent `.exe` overwrites inside MSI-managed folders.
4. **Phase 3 — Signing & Package Repositories (Production):**
   - Add Windows Authenticode signing (`.pfx` or Azure Trusted Signing) to eliminate SmartScreen warnings and unlock smooth `msiexec` auto-updates.
   - Add Apple Developer ID signing + notarization (`notarytool` + `stapler`) for macOS.
   - Automate submissions to `winget`, Homebrew Cask, and Flathub.
