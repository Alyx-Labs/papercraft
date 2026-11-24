# Iconography

PaperCraft uses a single outline icon set: [Lucide](https://lucide.dev/), vendored from the official `icons/` directory at release `1.72.0`.

Only the SVG files used by the app and the upstream ISC license are stored in `crates/ui-egui/assets/icons/lucide/`. The app embeds these assets at compile time and renders them through `egui_extras` as theme-tinted textures. Their 24×24 view box, 2 px outline, and rounded line caps and joins keep the controls visually consistent in light and dark themes.

## Adding or changing an icon

1. Add the official SVG from the pinned release to the local asset folder. Do not copy the full icon collection into the app.
2. Keep `currentColor`, the 24×24 view box, the outline weight, and rounded line caps and joins. The shared renderer applies the current UI color.
3. Add a semantic `Icon` variant or update its mapping in `crates/ui-egui/src/icons.rs`; UI code should use that registry instead of loading or drawing an icon itself.
4. Update this document if an asset needs a local adjustment or if the pinned Lucide release changes. Keep the upstream ISC license with the SVG assets.

All vendored SVG files are unmodified upstream assets.

The `mouse-pointer-2.svg` asset supplies the Select arrow in the canvas toolbar.
The `image.svg` asset supplies the Insert Image action button (`Icon::Image`) in the canvas toolbar.
The `link.svg` asset supplies the Insert / Edit Link action (`Icon::Link`) in the floating text editing dock.
The `folder.svg`, `folder-plus.svg`, `folder-input.svg`, `star.svg`, `tag.svg`, `list.svg`, `layout-grid.svg`, `arrow-up-down.svg`, `arrow-up.svg`, `arrow-down.svg`, `bookmark.svg`, `rotate-cw.svg`, `square-check.svg`, and `arrow-right-left.svg` assets supply Library organization (folders, favorites, tags, sorting, grid/list layout) and Page management (bookmarks, reordering, rotation, batch multi-select, page transfer/merge) controls.
The `folder-open.svg`, `folder-pen.svg`, `inbox.svg`, `chevron-down.svg`, and `ellipsis.svg` assets (together with the existing `chevron-right.svg`) supply the Library sidebar folder tree: the open folder, Rename Folder, Unfiled notebooks, expand/collapse toggles, and per-folder action menus.

Additional Lucide `1.72.0` outline assets for writing tools, shape styling, selection, rich text formatting, and stickers:

- `pencil-line.svg` (`Icon::Pencil`) — graphite Pencil writing tool
- `lasso.svg` (`Icon::Lasso`) — freeform lasso selection mode
- `pentagon.svg` (`Icon::PolygonShape`) — freeform/recognized polygon shape
- `paint-bucket.svg` (`Icon::FillBucket`) — shape fill color and opacity toggle
- `clipboard-paste.svg` (`Icon::Paste`) — paste selection across pages and notebooks
- `letter-b.svg` (`Icon::Bold`) — bold rich text span toggle
- `italic.svg` (`Icon::Italic`) — italic rich text span toggle
- `underline.svg` (`Icon::Underline`) — underline rich text span toggle
- `text-align-start.svg` (`Icon::AlignLeft`) — left-aligned text paragraph
- `text-align-center.svg` (`Icon::AlignCenter`) — center-aligned text paragraph
- `text-align-end.svg` (`Icon::AlignRight`) — right-aligned text paragraph
- `list-todo.svg` (`Icon::CheckList`) — interactive checkbox list toggle
- `sticker.svg` (`Icon::Sticker`) — saved stickers & reusable elements library
- `laser.svg` (`Icon::LaserPointer`) — ephemeral presentation laser pointer tool
- `chevron-up.svg` (`Icon::ChevronUp`) — expand/collapse toggle for the Pages sidebar active-page footer card
- `grip-horizontal.svg` (`Icon::GripHorizontal`) — top drag-handle grip on the floating color dock
- `type.svg` (`Icon::Text`) — Text Box insertion tool in the primary canvas toolbar
- `github.svg` (`Icon::GitHub`, vendored from `lucide-static` `0.577.0`, ISC license, as brand icons were removed in Lucide `1.0`) — discreet repository link icon in the Home library sidebar footer

