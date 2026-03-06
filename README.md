# cascii-terminal-decorator

Minimal `crossterm` TUI player for `cascii` frame output, reusing
`cascii-core-view` for frame parsing and animation state.

## What It Plays

- `frame_*.txt` files (text-only), loaded first when present
- `.cframe` files (full RGB per character), used as sidecars alongside `.txt` files or standalone when no `.txt` files exist
- When both `frame_*.txt` and matching `.cframe` files exist, pass `--color` to enable colored rendering

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

Requires the sibling `cascii-core-view` crate (`../cascii-core-view`).

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
```

Or via `cargo run`:

```bash
cargo run -- /path/to/frames --fps 30 --color
```

## Controls

| Key              | Action                   |
| ---------------- | ------------------------ |
| `q` / `Esc`      | Quit                     |
| `Space`          | Play / pause             |
| `Left` / `Right` | Step backward / forward  |
| `Home` / `End`   | Jump to first / last frame |
| `+` / `-`        | Increase / decrease FPS  |
| `l`              | Toggle loop / once       |

## License

MIT
