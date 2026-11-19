# cascii-terminal-decorator

A `crossterm` terminal player for `cascii` frame output, reusing
`cascii-core-view` for frame parsing and animation state.

## What It Plays

- `frame_*.txt` files (text-only), loaded first when present
- `.cframe` files (full RGB per character), used as sidecars alongside `.txt` files or standalone when no `.txt` files exist
- Cascii Studio `.bin` exports (`CFPK` full-fidelity cframe packs), passed directly as a playback path
- legacy packed multi-frame blobs (`.bin` or `.cframes`), passed directly or via `--packed`
- optional per-cell background-color extensions in either binary format when `--color` is enabled
- CSMV v1/v2 multi-size binaries, including text grids, truecolor, cell backgrounds and authored per-frame column maps
- Tiled manifests (JSON or gzip), individual gzip CSMV tiles, and the indexed multi-member gzip archive from [Adaptive Glyph Matrices](https://hjoncour.com/blog/15)

Color is enabled by default. Fixed-size animations fit by sampling their original grid. Multi-size animations fit by choosing the largest stored grid that fits the current frame in both columns and rows. The status bar uses one row when visible. If the smallest grid overflows, fit mode samples it down. A terminal uses the font and cell dimensions selected in the terminal emulator.

## Install

Linux builds require the ALSA development headers and `pkg-config`. On Debian or Ubuntu, install them with `sudo apt install libasound2-dev pkg-config`.

On macOS and Linux run

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

`Cargo.lock` pins the shared multi-size decoder used by this player.

## Responsive playback

```bash
casciit play frames.bin --paused
casciit play preload-20-70-v1-2863304f2599.bin.gz
casciit save preload-20-70-v1-2863304f2599.bin.gz ethereum
casciit play ethereum --columns 40
casciit play frames.bin --column-map
casciit play ethereum --transition logarithmic --direction up
```

Resize the terminal while paused to inspect different renditions of the same frame. The information bar shows the stored grid and selection mode. `--columns` selects the nearest generated width, with ties going to the smaller width. `--column-map` follows the authored width, falling back to the manifest default. Transitions run across the clip in either direction using the web player's linear, exponential or logarithmic curve. Explicit width, map and transition modes take precedence over automatic width selection. `--no-fit` starts at the authored width and crops overflow.

A local path or HTTP(S) asset URL works with `play` and `save`. Large remote indexed assets require a server that supports byte ranges. Playback reads the index and requested payloads. Tiled sources verify each tile's SHA-256 and retain up to four inflated tiles. Saving validates every frame and width, and copies referenced tiles for offline playback. When a folder contains several candidate animations, pass the desired file directly.

An adjacent `audio.mp3` follows play, pause, seek, loop and FPS changes. Remote multi-size sources can declare that sidecar in their manifest. Use `--no-audio` for silent playback. If the audio device is unavailable, the animation continues and the bar reports the audio status.

## Usage

```bash
# Validate and save the bundled example by folder name
casciit save test/familyguy

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
is optional: directories default to their final path component (`test/familyguy`
becomes `familyguy`), while binary files default to their filename without the
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
`--no-color` uses text with the project foreground and background colors for that session.
Legacy settings created before this default changed are migrated to color-on
once; a subsequent explicit `color off` choice is preserved.

Set the default playback speed:

```bash
casciit config set fps 30
```

FPS comes from `--fps`, the multi-size manifest, the project’s `details.toml`, or this saved default, in that order. FPS must be a positive whole number.

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
| `[` / `]`        | Previous / next generated width    |
| `a` / `p`        | Automatic fit / authored column map |
| `g`, digits, `Enter` | Go to a one-based frame within the active range. `Esc` cancels |
| `c`              | Cycle monochrome, project palette and available per-cell color |
| `m`              | Mute / unmute audio |
| `,` / `.`        | Lower / raise volume by 10% |

`--start` and `--end` set a playback range. `Home` and `End` jump to that
range's first and last frames.

## License

MIT
