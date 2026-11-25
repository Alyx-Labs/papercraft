# Feature Roadmap — PaperCraft

Tracking checklist of planned features to expand **PaperCraft** across handwriting, document management, productivity, and study.

> [!NOTE]
> **Initial Alpha Philosophy & Long-Term Direction**
>
> - **Concept validation & community feedback:** This initial alpha exists to validate the core handwriting experience, local-first + self-hosted tray-server workflow, and architecture together with early users.
> - **Extremely lightweight, no rush:** Features are prioritized deliberately so PaperCraft stays fast, minimal, and low-footprint rather than growing bloated.
> - **Future mobile ports (Android, iOS & tablets):** Once the core desktop/web experience stabilizes, the goal is to port thin native shells onto Android and iOS reusing the exact same `L0–L5` pure-Rust engine.
> - **AI-assisted contributions welcome:** Contributors are strongly encouraged to use AI coding assistants when tackling items from this roadmap—as long as all changes **strictly follow [`AGENTS.md`](../AGENTS.md)** and pass `cargo xtask ci` before submission.

> Legend: `[x]` Implemented · `[~]` Partially implemented · `[ ]` Pending
>
> Priority: **P1** core parity, next milestones · **P2** important, after P1 · **P3** long-term / heavy / optional

_Last audit: November 2026 (`v0.2.3` alpha) — statuses verified against the engine command registry (`papercraft-cli -- commands`), the document model in `papercraft-doc`, and the `papercraft-ui-egui` shell._

---

## 0. Competitive Gap Analysis

This audit compares PaperCraft against the feature set users expect from **market-leading tablet handwriting notebook apps**. Each capability is rated by how much it matters for daily note-taking, study, and PDF annotation, and by how well it fits PaperCraft's lightweight, local-first philosophy.

### 0.1 Where PaperCraft already matches or leads

| Area | PaperCraft today |
|---|---|
| Ink engine | Fountain, ball, brush, pencil, and highlighter pens with pressure, tilt, rotation, barrel buttons, and eraser end; per-pen percentage smoothing; pressure curves |
| Erasers | Stroke and precision erasers, plus a highlighter-only mode |
| Shapes | Draw & Hold recognition (line, rectangle, ellipse, triangle, arrow, polygon), fill colors and dashed/dotted borders |
| Selection | Rectangular and freeform lasso with move, resize, rotate, recolor, copy/cut/paste/duplicate across pages and notebooks |
| Page content | Rich text boxes (bold, italic, underline, highlight, font, size, color, alignment, bullets, checkboxes, URL links), images (picker, paste, drag-and-drop), stickers/elements, laser pointer, ruler |
| Paper | 11 built-in rulings and planners, custom image/PDF templates, sheet tints, page size & orientation presets |
| PDFs | Import as notebook or into an existing notebook on a protected background layer; crisp render, thumbnails, and export |
| Library | Nested folders, favorites, tags, sort, grid/list, title search, Trash recovery, bookmarks, page reorder/move/copy/merge, batch page actions |
| Export | `.png`, `.svg`, `.pdf`, `.pcraft`, and `.pcraftpkg` workspace backups |
| Sync | Self-hosted multi-account sync server (tray app + CLI), offline-first, Desktop + Web — **no vendor cloud required (a differentiator)** |
| Platforms | Windows, Linux, experimental macOS, and a fully client-side WebAssembly app with W3C pointer stylus support |

### 0.2 Gaps vs. leading handwriting apps

| Capability | Status | Priority | Section |
|---|---|---|---|
| Continuous vertical/horizontal page scrolling | Missing | **P1** | [2](#2-navigation-canvas--workspace) |
| Typed-text & PDF-text search inside notebooks | Missing (library searches titles only) | **P1** | [5](#5-study-search-audio--intelligence) |
| Notebook outline / table of contents (PDF + user-created) | Missing | **P1** | [1](#1-pdfs-import--documents), [2](#2-navigation-canvas--workspace) |
| Internal page links (link to page / outline entry) | Partial (external URLs only) | **P1** | [2](#2-navigation-canvas--workspace) |
| Read-only / viewing mode | Missing | **P1** | [2](#2-navigation-canvas--workspace) |
| Stylus-only input mode & touch gestures (two-finger tap undo, three-finger redo) | Partial (pinch zoom & palm-safe stylus) | **P1** | [2](#2-navigation-canvas--workspace) |
| Hybrid vector PDF export (keep original PDF vectors & text) | Missing | **P1** | [1](#1-pdfs-import--documents) |
| PDF text selection, copy & text-snapping highlighter | Missing | **P1** | [1](#1-pdfs-import--documents) |
| Zoom window (magnified writing strip for small handwriting) | Missing | **P2** | [4](#4-writing-tools-shapes--selection) |
| Scribble-to-erase gesture | Missing | **P2** | [4](#4-writing-tools-shapes--selection) |
| Presentation mode (full screen, hidden UI, external display) | Partial (laser pointer only) | **P2** | [2](#2-navigation-canvas--workspace) |
| Tabs / split view / multiple windows | Missing | **P2** | [2](#2-navigation-canvas--workspace) |
| Stroke-synchronized audio recording | Missing | **P2** | [5](#5-study-search-audio--intelligence) |
| Study tape & flashcards | Missing | **P2 / P3** | [5](#5-study-search-audio--intelligence) |
| Automatic cloud / WebDAV backups & version history | Partial (manual `.pcraftpkg` packages) | **P2** | [6](#6-collaboration-backup--ecosystem) |
| Printing | Missing | **P2** | [1](#1-pdfs-import--documents) |
| Handwriting recognition: search, convert to text, spellcheck | Missing | **P3** | [5](#5-study-search-audio--intelligence) |
| Math recognition & assistance | Missing | **P3** | [5](#5-study-search-audio--intelligence) |
| Optional AI assistant (summaries, Q&A over notes) | Missing | **P3** | [5](#5-study-search-audio--intelligence) |
| Share links, real-time collaboration & comments | Missing | **P3** | [6](#6-collaboration-backup--ecosystem) |
| Infinite whiteboard canvas | Missing | **P3** | [2](#2-navigation-canvas--workspace) |
| Document scanning & office document import | Missing | **P3** | [1](#1-pdfs-import--documents) |
| Native mobile/tablet apps | Missing | **P3** | [6](#6-collaboration-backup--ecosystem) |

> [!IMPORTANT]
> **Lightweight guardrails for heavy features.** Recognition (OCR, handwriting-to-text, math), AI, and audio features must be **opt-in, off by default, and feature-gated** so the default desktop and Web builds stay small. Prefer pure-Rust, on-device engines or the self-hosted server over vendor cloud APIs, keep model weights as optional downloads, and never block the ink pipeline.

### 0.3 Suggested milestone order

| Milestone | Focus | Items |
|---|---|---|
| **Next (`0.3.x`)** | Navigation & reading parity | Continuous scrolling · read-only mode · go-to-page · stylus-only mode & tap gestures · notebook outline + internal page links · PDF outline import |
| **Then (`0.4.x`)** | Find & annotate PDFs | In-notebook text search (text boxes + PDF text) · PDF text selection & snapping highlighter · hybrid vector PDF export · password-protected PDFs · printing |
| **Later (`0.5.x`)** | Writing comfort & study | Zoom window · scribble-to-erase · presentation mode · tabs/split view · study tape · audio recording · automatic backups & version history |
| **Long-term** | Intelligence, collaboration & mobile | Handwriting recognition & search · handwriting-to-text · math · opt-in AI · flashcards · share links & collaboration · whiteboards · scanning · mobile ports |

---

## 1. PDFs, Import & Documents

- [x] **Import PDF as a new notebook** — open a `.pdf` and convert each page into an annotatable sheet while preserving original aspect ratio and orientation.
- [x] **Import PDF pages into an existing notebook** — insert PDF pages after the active page with Undo/Redo support.
- [x] **Protected PDF background layer** — keep PDF content separate from annotations (not moved by the Select tool or erased by Clear Page).
- [x] **Full rendering of imported PDFs** — render PDFs crisply on the canvas, in thumbnails (sidebar and library), and on export (`.png`, `.svg`, `.pdf`).
- [x] **PDF persistence & sync** — store deduplicated PDF sources inside `.pcraft` (binary v2 and JSON v1) and maintain sync compatibility.
- [ ] **P1 · PDF Outline / Table of Contents** — extract bookmarks/chapters from imported PDFs for quick navigation in the pages sidebar.
- [ ] **P1 · Native PDF text selection & copy** — allow selecting, copying, and highlighting embedded text directly inside PDFs, including a highlighter that snaps to text lines.
- [ ] **P1 · Hybrid vector PDF export** — preserve original PDF vectors and searchable text when re-exporting instead of rasterizing the base layer; option to export annotated pages only.
- [ ] **P2 · Password-protected PDFs** — prompt for a password when importing encrypted files.
- [ ] **P2 · Printing** — print the current page, a page range, or the whole notebook through the system print dialog (Desktop) and browser print (Web).
- [ ] **P2 · Export options** — choose page range, include/exclude paper template and PDF background, and flatten vs. keep layers when exporting.
- [ ] **P3 · Import other documents** — import office documents and presentations (converted to PDF pages) and multi-image imports as pages.
- [ ] **P3 · Document scanning** — capture paper pages via camera with edge detection and perspective correction (primarily for future mobile ports).

---

## 2. Navigation, Canvas & Workspace

- [x] **Page size & orientation picker** — presets such as Standard, A4, A5, Letter, 16:9, and Portrait/Landscape toggle when creating or editing pages.
- [x] **Pinch zoom & palm-safe stylus input** — two-finger pinch zoom and pan, cursor-anchored `Ctrl` + wheel zoom, fit page, and stylus strokes that keep drawing while a palm rests on the screen.
- [x] **External URL links in text boxes** — attach `https://`, `http://`, or `mailto:` links to text items and auto-detect URLs in text.
- [ ] **P1 · Continuous page scrolling** — scroll vertically or horizontally through the entire notebook without switching page by page (with a setting to keep single-page mode).
- [ ] **P1 · Read-only mode** — lock stroke creation so you can read, zoom, scroll, and click links without accidental marks.
- [ ] **P1 · Go to page & page indicator** — jump directly to a page number and show the current/total page count on the canvas.
- [ ] **P1 · Notebook outline** — add any page to a user-created outline (named entries, nesting) shown in the pages sidebar next to imported PDF outlines.
- [ ] **P1 · Internal page hyperlinks** — create links inside text boxes (or on a selection) pointing to another page or outline entry in the same notebook (ideal for planners and indexes).
- [ ] **P1 · Stylus-only drawing & touch gestures** — "stylus-only draws" mode (finger only pans/zooms), two-finger tap to undo, three-finger tap to redo, and left-/right-handed ergonomics adjustments.
- [ ] **P2 · Presentation mode** — full-screen canvas with hidden UI, keyboard/clicker page navigation, laser pointer, and optional mirroring of the clean page to a second window or display.
- [ ] **P2 · Tabs & split view / multiple windows** — keep several notebooks open as tabs and view two notebooks or two pages side by side.
- [ ] **P2 · Recent notebooks & quick note** — a recent-notebooks shelf on the Home screen and a one-click quick note action.
- [ ] **P3 · Infinite whiteboard canvas** — an unbounded canvas notebook type for brainstorming and diagrams.

---

## 3. Library & Page Management

- [x] **Folders & subfolders in Library** — organize notebooks in nested directories on the Home screen.
- [x] **Favorites & Tags** — star notebooks and filter by labels.
- [x] **Sorting & List view** — sort by modified date, created date, or name, and switch between grid and list views.
- [x] **Library title search** — filter notebooks by title from the Home screen.
- [x] **Trash recovery** — restore deleted notebooks and clean up expired items automatically.
- [x] **Reorder pages in sidebar** — drag and drop thumbnails (or move up/down) in the pages sidebar.
- [x] **Move or copy pages between notebooks** — transfer selected pages to another notebook or merge notebooks.
- [x] **Bookmarks (favorite pages)** — bookmark important pages inside a notebook and filter the sidebar by favorites.
- [x] **Batch page actions** — rotate individual pages, apply a template across the whole notebook, and multi-select pages to duplicate/delete/export.
- [x] **User-created custom templates** — save images or PDFs as reusable paper templates when creating pages.
- [x] **New built-in paper templates** — isometric grid, music staff, weekly/monthly planner, and checklist sheets.
- [ ] **P2 · Insert page before/after with template choice** — insert a page at any position from the sidebar context menu, choosing template, size, or an image/PDF page.
- [ ] **P2 · Custom cover images** — use an image as a notebook cover in addition to hardcover colors.
- [ ] **P3 · Linked planners & dated templates** — generate planner notebooks (daily/weekly/monthly) with pre-linked calendar navigation pages.

---

## 4. Writing Tools, Shapes & Selection

- [x] **Freehand shape recognition ("Draw & Hold")** — when drawing a circle, ellipse, rectangle, triangle, arrow, or polygon and holding at the end, automatically convert the stroke into a clean geometric shape.
- [x] **Freeform Lasso** — selection by a hand-drawn outline in addition to the current rectangular box.
- [x] **Copy, cut, paste & duplicate any selection** — extend `Ctrl+C`/`Ctrl+X`/`Ctrl+V` to strokes, shapes, images, and mixed selections (including across pages and notebooks).
- [x] **Pencil tool** — new instrument with graphite texture and tilt-sensitive shading.
- [x] **Shape fill & line styles** — fill shapes with color/opacity and support dashed or dotted borders.
- [x] **"Erase highlighter only" mode** — optional eraser filter to remove only highlights without erasing ink or notes underneath.
- [x] **Rich text in text boxes** — span-level formatting (bold, italic, underline, highlight, font, size, color), alignment, and bullet/checkbox lists.
- [x] **Stickers / Elements** — library of user-saved stamps, figures, and diagrams for quick reuse.
- [x] **Laser pointer** — temporary dot or trail that fades after a few seconds for presentations and lessons.
- [x] **Toolbar style favorites** — save quick color + thickness shortcuts directly accessible in the top bar.
- [ ] **P2 · Zoom window** — a docked magnified writing strip that lets you write large and places small, neat handwriting on the page, with auto-advance to the next line.
- [ ] **P2 · Scribble-to-erase** — scribbling back and forth over ink erases it without switching tools (toggleable).
- [ ] **P2 · Eyedropper** — pick an ink color directly from the canvas, images, or PDF background.
- [ ] **P2 · Image crop & mask** — crop, rotate, and round-corner placed images.
- [ ] **P2 · More text fonts** — add a serif and a handwriting-style font to the existing Sans and Monospace families (embedded, license-compatible).
- [ ] **P3 · Highlighter & pen auto-straighten** — optional auto-straight highlighter lines and snapping of shapes to the paper grid.
- [ ] **P3 · Additional instruments** — protractor/angle tool and set squares alongside the ruler.

---

## 5. Study, Search, Audio & Intelligence

- [ ] **P1 · In-notebook text search** — search typed text (`TextItem`) and extracted PDF text across a notebook or the entire library, with highlighted hits and next/previous navigation.
- [ ] **P2 · Stroke-synchronized audio recording** — record audio while writing and tap a stroke later to hear the matching segment of the recording; replay mode that redraws ink in sync.
- [ ] **P2 · Study tape for active recall** — clickable masking strips on the page to hide answers and test memorization.
- [ ] **P3 · Handwriting indexing & search (OCR)** — find handwritten words directly via search (opt-in, on-device or self-hosted engine).
- [ ] **P3 · Handwriting-to-text conversion** — convert selected handwritten strokes into a text box.
- [ ] **P3 · Handwriting spellcheck & word completion** — underline misspelled handwritten words and suggest corrections (opt-in, depends on recognition).
- [ ] **P3 · Math recognition** — convert handwritten math into formatted equations and optionally check or evaluate expressions.
- [ ] **P3 · Flashcards / Study sets** — create spaced-repetition study cards from notebook selections.
- [ ] **P3 · Optional AI assistant** — summarize pages, answer questions about a notebook, or generate study cards using a user-configured, self-hosted or local model (off by default, never required).

---

## 6. Collaboration, Backup & Ecosystem

- [x] **System-tray self-hosted server control app (`papercraft-server`)** — standalone desktop executable with tray menu, minimalist control window, folder picker, copyable local/LAN URLs, and `server_control_config.json` persistence.
- [x] **WebAssembly W3C `PointerEvent` stylus bridge** — hardware pen pressure, tilt, rotation, barrel buttons, and inverted eraser end directly in browsers on Windows, Linux, macOS, and tablets.
- [x] **Linux & macOS desktop clipboard & file dialog parity** — native text/image clipboard (`wl-clipboard`/`xclip`/`xsel` on Linux, `pbpaste`/`pbcopy`/`osascript` on macOS), file dialogs, and Linux `/sys/class/input` stylus detection.
- [x] **Workspace backup packages** — export and import complete `.pcraftpkg` packages with notebooks and studio preferences.
- [ ] **P1 · Native Linux & macOS low-level desktop stylus monitors** — wire XInput2 / Wayland `zwp_tablet_v2` and AppKit `NSEvent` hardware monitors into `papercraft`, and complete community verification on physical macOS hardware.
- [ ] **P2 · Scheduled automatic backups** — periodic `.pcraftpkg` / PDF backups to a local folder, WebDAV, or other cloud storage targets.
- [ ] **P2 · Notebook version history** — keep restorable snapshots of notebooks (locally and on the sync server) to recover from accidental edits.
- [ ] **P2 · Accessibility** — screen-reader labels, full keyboard navigation, and high-contrast checks for toolbar, sidebar, and modals.
- [ ] **P2 · Keyboard shortcut reference & customization** — an in-app shortcut sheet with remappable keys.
- [ ] **P3 · Link sharing** — publish or share notebooks with read or edit permissions on the server.
- [ ] **P3 · Real-time collaboration & comments** — simultaneous editing and per-page comment threads between users.
- [ ] **P3 · Native mobile apps** — dedicated support for tablets and phones with active stylus.
