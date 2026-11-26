# Attribution & Asset Licensing

This document records the licensing status of PaperCraft's source code, brand assets, and bundled third-party materials.

## Source Code

PaperCraft's original source code and repository documentation are dual-licensed under your choice of either:

- **MIT License** — [LICENSE-MIT](LICENSE-MIT)
- **Apache License, Version 2.0** — [LICENSE-APACHE](LICENSE-APACHE)

See [NOTICE](NOTICE) for the top-level copyright and redistribution notice.

---

## Brand Assets & Trademarks (Not Open Source)

The **Alyx Labs**, **PaperCraft**, **Paper Craft**, and **PaperCraft Server** names, wordmarks, official domains (`papercraft.ink`, `web.papercraft.ink`), bundle identifiers (`com.alyxlabs.*`), and visual brand assets below are **proprietary trademarks and brand assets of Alyx Labs**. They are **not** open source and are governed strictly by [apps/papercraft/assets/branding/LICENSE-brand.txt](apps/papercraft/assets/branding/LICENSE-brand.txt):

| Asset / Mark | Location | Terms |
|---|---|---|
| Canonical logo & derived desktop/web icons | [`apps/papercraft/assets/branding/`](apps/papercraft/assets/branding/) (`logo.png`, `papercraft-*.png`, `papercraft-256.rgba`, `papercraft.ico`, `papercraft.icns`) | Proprietary brand asset — see [`LICENSE-brand.txt`](apps/papercraft/assets/branding/LICENSE-brand.txt) |
| Welcome banner brand illustration | [`crates/ui-egui/assets/illustrations/welcome-sea.png`](crates/ui-egui/assets/illustrations/welcome-sea.png) | Proprietary brand asset — see [`LICENSE-brand.txt`](apps/papercraft/assets/branding/LICENSE-brand.txt) |

---

## Bundled Third-Party Assets

All third-party assets embedded in the repository or compiled binaries retain their respective upstream licenses:

| Component | Repository Path / Crate | Author / Source | License | License Notice |
|---|---|---|---|---|
| **Roboto** font family (`Regular`, `Bold`, `Italic`, `BoldItalic`) | `crates/compose/assets/fonts/Roboto-*.ttf` | Google / Roboto authors | Apache-2.0 | [`crates/compose/assets/fonts/LICENSE`](crates/compose/assets/fonts/LICENSE) |
| **Bricolage Grotesque** (`Bold`) | `crates/ui-egui/assets/fonts/BricolageGrotesque-Bold.ttf` | The Bricolage Grotesque Project Authors (`https://github.com/ateliertriay/bricolage`) | SIL Open Font License 1.1 | [`crates/ui-egui/assets/fonts/OFL.txt`](crates/ui-egui/assets/fonts/OFL.txt) |
| **Lucide** outline SVG icons (`lucide-static` v1.72.0) | `crates/ui-egui/assets/icons/lucide/*.svg` | Lucide Icons and Contributors (`https://lucide.dev/`) | ISC License | [`crates/ui-egui/assets/icons/lucide/LICENSE`](crates/ui-egui/assets/icons/lucide/LICENSE), [`docs/ICONOGRAPHY.md`](docs/ICONOGRAPHY.md) |
| **Feather**-derived icons within Lucide | Listed inside `crates/ui-egui/assets/icons/lucide/LICENSE` | Cole Bemis (Feather project) | MIT License | [`crates/ui-egui/assets/icons/lucide/LICENSE`](crates/ui-egui/assets/icons/lucide/LICENSE) |
| **Ubuntu-Light** font | Embedded via `epaint_default_fonts` v0.37.0 (`egui` / `eframe` `default_fonts` feature) | Canonical Ltd. / Ubuntu font contributors (`https://design.ubuntu.com/font`) | Ubuntu Font Licence 1.0 | Upstream `epaint_default_fonts` `UFL.txt` |
| **Hack-Regular** font | Embedded via `epaint_default_fonts` v0.37.0 | Source Foundry Authors (`https://github.com/source-foundry/Hack`), Bitstream Vera & DejaVu contributors | MIT License & Bitstream Vera License | Upstream `epaint_default_fonts` `Hack-Regular.txt` |
| **egui-icons** (subset of `emoji-icon-font`) | Embedded via `epaint_default_fonts` v0.37.0 | John Slegers (`https://github.com/jslegers/emoji-icon-font`) | MIT License | Upstream `epaint_default_fonts` `emoji-icon-font-mit-license.txt` |

---

## Rust Crate Dependencies

All Cargo dependencies declared in [Cargo.toml](Cargo.toml) and [Cargo.lock](Cargo.lock) are governed by their own open-source licenses and verified against the allowed license policy in [deny.toml](deny.toml) (`cargo deny check`).
