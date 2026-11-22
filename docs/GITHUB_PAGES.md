# PaperCraft Web (WebAssembly & GitHub Pages)

A 100% client-side web build of PaperCraft powered by **Rust + WebAssembly (WASM)** running directly in the browser.

- **Zero backend required**: Runs entirely in the browser with no Node.js, Python, or database server.
- **Stylus and pen input**: Supports pressure, tilt, and touch via native browser `PointerEvent` events.
- **Hardware-accelerated graphics**: Renders via WebGL / WebGPU on an HTML5 `<canvas>`.

---

## Automatic Deployment with GitHub Pages

PaperCraft includes [`.github/workflows/pages.yml`](../.github/workflows/pages.yml) to build `apps/papercraft-web` with Trunk and publish it automatically to **GitHub Pages** whenever web/workspace code changes on `main` (or when triggered manually via **Run workflow**).

### One-time repository setup on GitHub

1. Open your repository on GitHub and go to **Settings → Pages**.
2. Under **Build and deployment → Source**, select **GitHub Actions**.
3. Trigger the **Deploy Web (GitHub Pages)** workflow from the **Actions** tab (or push a change to `main`).
4. Once complete, the app will be live at **`https://alyxhard.github.io/papercraft/`** (and GitHub automatically displays the deployment environment under **Deployments** in the repository sidebar).

> [!NOTE]
> The workflow passes `--public-url ./` into `trunk build --release`, so asset paths resolve relatively and work on project subpaths (`/papercraft/`), custom domains (`/`), and reverse proxies without needing a rebuild when switching domains.

---

## Local Prerequisites

Install the WebAssembly target and [Trunk](https://trunkrs.dev/) (both pure Rust tools):

```powershell
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
```

---

## Development vs Production Bundle

> For Desktop/Windows, Sync Server, and full workspace commands, see the [Command Reference](COMMANDS.md).

### 1. Run Web in Development Mode (Hot Reload)

Run a local development server from the repository root:

```powershell
cargo xtask web
```

*(Equivalent to `cd apps/papercraft-web && trunk serve`.)*  
Open **`http://127.0.0.1:8080`** in your browser. Pass `--address 0.0.0.0` to test from other devices on your local network, or `--open` to launch the browser automatically.

### 2. Preview the Release Build Locally

Test the optimized WebAssembly release bundle locally without auto-reload:

```powershell
cargo xtask web --release --no-autoreload
```

*(Equivalent to `cd apps/papercraft-web && trunk serve --release --no-autoreload`.)*

### 3. Generate the Production Static Bundle (`dist/`)

Build the static release files into `dist/` (ignored by Git):

```powershell
cargo xtask web-build
```

*(Equivalent to `cd apps/papercraft-web && trunk build --release`.)*

Trunk compiles the Rust crate with release optimizations and writes `index.html`, `.js`, and `.wasm` into `dist/` (configured in [`apps/papercraft-web/Trunk.toml`](../apps/papercraft-web/Trunk.toml)).

If hosting `dist/` under a subdirectory on any static web server (e.g. `https://example.com/papercraft/`), build with `--public-url`:

```powershell
cd apps/papercraft-web && trunk build --release --public-url /papercraft/
```
