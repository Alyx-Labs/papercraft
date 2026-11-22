# PaperCraft branding assets

`logo.png` is the source artwork. Replace it with another PNG and run
`scripts/generate-branding.cmd` from a terminal, or run `cargo xtask branding`
from the workspace, to regenerate all derived files.
The command fits the artwork inside a transparent square, preserving its
aspect ratio, and writes app-icon PNGs from 16×16 through 1024×1024, a
multi-size Windows `.ico`, a macOS `.icns`, and the 256×256 RGBA bytes used by
the desktop window icon. The Windows `.ico` is embedded in the executable by
`build.rs`.

The generator is part of the Rust workspace and needs no separate image tools.
