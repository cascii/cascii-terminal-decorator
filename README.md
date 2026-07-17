# cascii-terminal-decorator

Minimal `crossterm` TUI player for `cascii` frame output, reusing
`cascii-core-view` for frame parsing and animation state.

## What It Plays

- `frame_*.txt` files (text-only), loaded first when present
- `.cframe` files (full RGB per character), used as sidecars alongside `.txt` files or standalone when no `.txt` files exist
- packed multi-frame blobs via `--packed` or by passing the blob file directly: CFPK `.bin` archives (the "Export packed cframes" output of cascii-studio) and legacy headerless packed `.cframes`, including the optional per-cell background-color extension when `--color` is enabled
- When both `frame_*.txt` and matching `.cframe` files exist, pass `--color` to enable colored rendering
- Frames are scaled down automatically to fit the terminal while preserving their character-grid aspect ratio; smaller frames are not enlarged

## Install

```bash
# Build and install as `casciit` to /usr/local/bin
./install.sh

# Or install to a custom directory
INSTALL_DIR=~/.local/bin ./install.sh
```

## Build

```bash
cargo build --release
```

`cascii-core-view` is resolved from crates.io.

## Usage

```bash
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

# Play a cascii-studio CFPK export directly (blob file as the positional argument)
casciit /path/to/export_cframes.bin --color

# Play a subsection of an animation (normalized positions from 0.0 to 1.0)
casciit /path/to/frames --start 0.25 --end 0.75

# Keep native frame dimensions and crop anything outside the terminal
casciit /path/to/frames --no-fit
```

Or via `cargo run`:

```bash
cargo run -- /path/to/frames --fps 30 --color
```

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
