# cascii-terminal-decorator

Minimal `crossterm` TUI player for `cascii` frame output, reusing
`cascii-core-view` for frame parsing and animation state.

## What It Plays

- `.cframe` files (preferred, full RGB per character)
- `frame_*.txt` files (text-only), with optional same-name `.cframe` sidecars

## Build

```bash
cargo build
```

## Run

```bash
# Play frames in current directory
cargo run -- .

# Play frames in a specific directory at 30 FPS
cargo run -- /path/to/frames --fps 30

# Play once (no loop)
cargo run -- /path/to/frames --once
```

## Controls

- `q` / `Esc`: quit
- `Space`: play/pause
- `Left` / `Right`: step backward/forward
- `Home` / `End`: jump to first/last frame
- `+` / `-`: increase/decrease FPS
- `l`: toggle loop/once
