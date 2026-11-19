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

# Play frames in current directory (default 30 FPS, looping)
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

Binary blobs do not contain playback FPS metadata, so they use the configured
default FPS. Pass `--fps` to override it for one playback.

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

Choose whether playback uses per-cell color by default:

```bash
casciit config set color on
casciit config set color off
```

The default is color (`on`). For one playback, `--color` forces color and
`--no-color` forces monochrome without changing the saved preference.
Legacy settings created before this default changed are migrated to color-on
once; a subsequent explicit `color off` choice is preserved.

Set the default playback speed:

```bash
casciit config set fps 30
```

The FPS precedence is `--fps`, then a project’s `details.toml`, then this saved
default. FPS must be a positive whole number.

Choose whether frames fit within the terminal by default:

```bash
casciit config set fit on
casciit config set fit off
```

Fit defaults to `on`. For one playback, `--fit` forces fitting and `--no-fit`
keeps native frame dimensions without changing the saved preference.

Configure looping and the information bar:

```bash
casciit config set loop on
casciit config set loop off
casciit config set hide-bar on
casciit config set hide-bar off
```

Looping defaults to `on`, and hiding the entire bar defaults to `off`.
`--loop`/`--once` and `--show-bar`/`--hide-bar` override those settings for one
playback.

Each bar field can also be shown or hidden independently:

```bash
casciit config set bar frame off
casciit config set bar fps off
casciit config set bar controls off
```

Available fields are `frame`, `range`, `state`, `fps`, `loop`, `fit`, `color`,
`audio`, and `controls`. `on` means visible and `off` means hidden. Every field
is visible by default when the bar itself is not hidden.

`settings.json` contains:

```json
{
  "name": "casciit",
  "version": "0.3.0",
  "binary_path": "/usr/local/bin/casciit",
  "save_path": "/home/user/.local/share/casciit/animations",
  "color": true,
  "fps": 30,
  "fit": true,
  "loop": true,
  "hide_bar": false,
  "bar": {
    "frame": true,
    "range": true,
    "state": true,
    "fps": true,
    "loop": true,
    "fit": true,
    "color": true,
    "audio": true,
    "controls": true
  },
  "schema_version": 2
}
```

`name`, `version`, and `binary_path` are refreshed automatically when the app
is upgraded or moved. User-configurable playback and bar preferences are
preserved.

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
