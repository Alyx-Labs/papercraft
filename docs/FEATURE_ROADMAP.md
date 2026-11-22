# Feature Roadmap — PaperCraft

Tracking checklist of planned features to expand **PaperCraft** across handwriting, document management, productivity, and study.

> [!NOTE]
> **Initial Alpha Philosophy & Long-Term Direction**
>
> - **Concept validation & community feedback:** This initial alpha exists to validate the core handwriting experience, local-first + self-hosted tray-server workflow, and architecture together with early users.
> - **Extremely lightweight, no rush:** Features are prioritized deliberately so PaperCraft stays fast, minimal, and low-footprint rather than growing bloated.
> - **Future mobile ports (Android, iOS & tablets):** Once the core desktop/web experience stabilizes, the goal is to port thin native shells onto Android and iOS reusing the exact same `L0–L5` pure-Rust engine.
> - **AI-assisted contributions welcome:** Contributors are strongly encouraged to use AI coding assistants when tackling items from this roadmap—as long as all changes **strictly follow [`AGENTS.md`](../AGENTS.md)** and pass `cargo xtask ci` before submission.

> Legend: `[x]` Implemented · `[ ]` Pending

---

## 1. PDFs, Import & Documents

- [x] **Import PDF as a new notebook** — open a `.pdf` and convert each page into an annotatable sheet while preserving original aspect ratio and orientation.
- [x] **Import PDF pages into an existing notebook** — insert PDF pages after the active page with Undo/Redo support.
- [x] **Protected PDF background layer** — keep PDF content separate from annotations (not moved by the Select tool or erased by Clear Page).
- [x] **Full rendering of imported PDFs** — render PDFs crisply on the canvas, in thumbnails (sidebar and library), and on export (`.png`, `.svg`, `.pdf`).
- [x] **PDF persistence & sync** — store deduplicated PDF sources inside `.pcraft` (binary v2 and JSON v1) and maintain sync compatibility.
- [ ] **PDF Outline / Table of Contents** — extract bookmarks/chapters from imported PDFs for quick navigation.
- [ ] **Native PDF text selection & copy** — allow selecting, copying, and highlighting embedded text directly inside PDFs.
- [ ] **Password-protected PDFs** — prompt for a password when importing encrypted files.
- [ ] **Hybrid vector PDF export** — preserve original PDF vectors and searchable text when re-exporting instead of rasterizing the base layer.
- [ ] **Import other documents & scanning** — import office documents / presentations or capture pages via camera.

---

## 2. Navigation, Canvas & Workspace

- [ ] **Continuous page scrolling** — scroll vertically or horizontally through the entire notebook without switching page by page.
- [ ] **Split view / multiple windows** — view two different notebooks or two different pages side by side.
- [ ] **Read-only mode** — lock stroke creation so you can read, zoom, scroll, and click links without accidental marks.
- [ ] **Internal page hyperlinks** — create links inside text boxes pointing to another page in the same notebook (ideal for planners).
- [x] **Page size & orientation picker** — presets such as Standard, A4, A5, Letter, 16:9, and Portrait/Landscape toggle when creating or editing pages.
- [ ] **Extra touch & ergonomics settings** — "stylus-only draws" mode (finger only pans/navigates) and left-/right-handed ergonomics adjustments.

---

## 3. Library & Page Management

- [x] **Folders & subfolders in Library** — organize notebooks in nested directories on the Home screen.
- [x] **Favorites & Tags** — star notebooks and filter by labels.
- [x] **Sorting & List view** — sort by modified date, created date, or name, and switch between grid and list views.
- [x] **Reorder pages in sidebar** — drag and drop thumbnails (or move up/down) in the pages sidebar.
- [x] **Move or copy pages between notebooks** — transfer selected pages to another notebook or merge notebooks.
- [x] **Bookmarks (favorite pages)** — bookmark important pages inside a notebook and filter the sidebar by favorites.
- [x] **Batch page actions** — rotate individual pages, apply a template across the whole notebook, and multi-select pages to duplicate/delete/export.
- [x] **User-created custom templates** — save images or PDFs as reusable paper templates when creating pages.
- [x] **New built-in paper templates** — isometric grid, music staff, weekly/monthly planner, and checklist sheets.

---

## 4. Writing Tools, Shapes & Selection

- [x] **Freehand shape recognition ("Draw & Hold")** — when drawing a circle, ellipse, rectangle, triangle, arrow, or polygon and holding at the end, automatically convert the stroke into a clean geometric shape.
- [x] **Freeform Lasso** — selection by a hand-drawn outline in addition to the current rectangular box.
- [x] **Copy, cut, paste & duplicate any selection** — extend `Ctrl+C`/`Ctrl+X`/`Ctrl+V` to strokes, shapes, images, and mixed selections (including across pages and notebooks).
- [x] **Pencil tool** — new instrument with graphite texture and tilt-sensitive shading.
- [x] **Shape fill & line styles** — fill shapes with color/opacity and support dashed or dotted borders.
- [x] **"Erase highlighter only" mode** — optional eraser filter to remove only highlights without erasing ink or notes underneath.
- [x] **Rich text in text boxes** — span-level formatting (bold, italic, underline, highlight), alignment, and bullet/checkbox lists.
- [x] **Stickers / Elements** — library of user-saved stamps, figures, and diagrams for quick reuse.
- [x] **Laser pointer** — temporary dot or trail that fades after a few seconds for presentations and lessons.
- [x] **Toolbar style favorites** — save quick color + thickness shortcuts directly accessible in the top bar.

---

## 5. Study, Search, Audio & Intelligence

- [ ] **In-page text search** — search typed text (`TextItem`) and extracted PDF text across a notebook or the entire library.
- [ ] **Handwriting indexing & search (OCR)** — find handwritten words directly via search.
- [ ] **Handwriting-to-text & math conversion** — convert selected handwritten strokes into a text box or formatted equation.
- [ ] **Stroke-synchronized audio recording** — record audio while writing and tap a stroke later to hear the matching segment of the recording.
- [ ] **Study tape for active recall** — clickable masking strips on the page to hide answers and test memorization.
- [ ] **Flashcards / Study sets** — create spaced-repetition study cards inside notebooks.

---

## 6. Collaboration, Backup & Ecosystem

- [x] **System-tray self-hosted server control app (`papercraft-server`)** — standalone desktop executable with tray menu, minimalist control window, folder picker, copyable local/LAN URLs, and `server_control_config.json` persistence.
- [x] **WebAssembly W3C `PointerEvent` stylus bridge** — hardware pen pressure, tilt, rotation, barrel buttons, and inverted eraser end directly in browsers on Windows, Linux, macOS, and tablets.
- [x] **Linux & macOS desktop clipboard & file dialog parity** — native text/image clipboard (`wl-clipboard`/`xclip`/`xsel` on Linux, `pbpaste`/`pbcopy`/`osascript` on macOS), file dialogs, and Linux `/sys/class/input` stylus detection.
- [ ] **Native Linux & macOS low-level desktop stylus monitors** — wire XInput2 / Wayland `zwp_tablet_v2` and AppKit `NSEvent` hardware monitors into `papercraft`, and complete community verification on physical macOS hardware.
- [ ] **Link sharing** — publish or share notebooks with read or edit permissions on the server.
- [ ] **Real-time collaboration & comments** — simultaneous editing and per-page comment threads between users.
- [ ] **Scheduled external cloud backups** — automated export/backup to cloud storage services and WebDAV.
- [ ] **Native mobile apps** — dedicated support for tablets and phones with active stylus.
