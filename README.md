# cascii-terminal-decorator

Minimal `crossterm` TUI player for `cascii` frame output, reusing
`cascii-core-view` for frame parsing and animation state.

## What It Plays

- `frame_*.txt` files (text-only), loaded first when present
- `.cframe` files (full RGB per character), used as sidecars alongside `.txt` files or standalone when no `.txt` files exist
- Cascii Studio `.bin` exports (`CFPK` full-fidelity cframe packs), passed directly as a playback path
- legacy packed multi-frame blobs (`.bin` or `.cframes`), passed directly or via `--packed`
- optional per-cell background-color extensions in either binary format when `--color` is enabled
- When both `frame_*.txt` and matching `.cframe` files exist, pass `--color` to enable colored rendering
- Frames are scaled down automatically to fit the terminal while preserving their character-grid aspect ratio; smaller frames are not enlarged

## Install

macOS and Linux:

```bash
# Build and install as `casciit` to /usr/local/bin
./install.sh

# Or install to a custom directory
INSTALL_DIR=~/.local/bin ./install.sh
```

Windows (PowerShell):

```powershell
# Builds and installs to %LOCALAPPDATA%\Programs\casciit\bin
.\install.ps1

# Or install to a custom directory
.\install.ps1 -InstallDir "C:\Tools\casciit"
```

The installers also initialize `settings.json`. The program checks the settings on every run, so `cargo install`, a package manager, or a manually copied binary will initialize it on first use as well.

## Build

```bash
cargo build --release
```

`cascii-core-view` is resolved from crates.io.

## Usage

```bash
# Validate and save using the source folder name (`eth`)
casciit save test/eth

# Save a frame directory under an easy-to-remember name
casciit save /path/to/frames family-guy

# Play a saved animation by name
casciit play family-guy

# Play a Cascii Studio binary export directly
casciit /path/to/family-guy_cframes.bin --color

# Save that binary and play it later by name
casciit save /path/to/family-guy_cframes.bin family-guy-binary
casciit play family-guy-binary --color

# Play frames in current directory (default 24 FPS, looping)
casciit .

# Play frames in a specific directory at 30 FPS
casciit /path/to/frames --fps 30

# Play once (no loop)
casciit /path/to/frames --once

# Enable colored rendering from .cframe data
casciit /path/to/frames --color

# Play a packed multi-frame blob (relative to the frame directory)
casciit /path/to/frames --packed animation.cframes --color

# Play a subsection of an animation (normalized positions from 0.0 to 1.0)
casciit /path/to/frames --start 0.25 --end 0.75

# Keep native frame dimensions and crop anything outside the terminal
casciit /path/to/frames --no-fit
```

The original `casciit <path>` form remains supported for both directories and
binary files. The explicit `casciit play <path-or-name>` form is useful in
scripts and when playing a saved animation. Saving copies a source file or
directory into the library and refuses to overwrite an existing name. `NAME`
is optional: directories default to their final path component (`test/eth`
becomes `eth`), while binary files default to their filename without the
extension. Before creating the saved copy, `casciit` fully loads the loose
frames or packed binary to confirm that the content is readable. When a saved
item contains one `.bin` or `.cframes` file and no loose frames, that binary is
selected automatically.

Binary blobs do not contain playback FPS metadata, so they use 24 FPS by
default. Pass `--fps` to override it.

Or via `cargo run`:

```bash
cargo run -- /path/to/frames --fps 30 --color
```

## Settings

Show the current settings or locate the file:

```bash
casciit config
casciit config path
```

Change the saved-animation library (relative paths are converted to absolute paths):

```bash
casciit config set save-path /path/to/animation-library
```

`settings.json` contains:

```json
{
  "name": "casciit",
  "version": "0.2.0",
  "binary_path": "/usr/local/bin/casciit",
  "save_path": "/home/user/.local/share/casciit/animations"
}
```

`name`, `version`, and `binary_path` are refreshed automatically when the app is upgraded or moved. A custom `save_path` is preserved.

Default locations follow each operating system's conventions:

| Platform | `settings.json` | Saved animations |
| --- | --- | --- |
| Linux | `$XDG_CONFIG_HOME/casciit/settings.json` or `~/.config/casciit/settings.json` | `$XDG_DATA_HOME/casciit/animations` or `~/.local/share/casciit/animations` |
| macOS | `~/Library/Application Support/com.cascii.casciit/settings.json` | `~/Library/Application Support/com.cascii.casciit/animations` |
| Windows | `%APPDATA%\cascii\casciit\config\settings.json` | `%LOCALAPPDATA%\cascii\casciit\data\animations` |

## Controls

| Key              | Action                            |
| ---------------- | --------------------------------- |
| `q` / `Esc`      | Quit                              |
| `Space`          | Play / pause                      |
| `Left` / `Right` | Step backward / forward           |
| `Home` / `End`   | Jump to active range start / end  |
| `+` / `-`        | Increase / decrease FPS           |
| `f`              | Toggle fit-to-terminal             |
| `l`              | Toggle loop / once                |

`--start` and `--end` set a playback range. `Home` and `End` jump to that
range's first and last frames.

## License

MIT
