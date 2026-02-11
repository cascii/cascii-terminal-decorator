use std::cmp::min;
use std::fs;
use std::io::{self, Stdout, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use cascii_core_view::{
    AnimationController, Frame, FrameFile, LoopMode, parse_cframe, parse_cframe_text,
};
use clap::Parser;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Minimal crossterm player for cascii .cframe/.txt frames"
)]
struct Args {
    /// Directory containing frame files (frame_*.cframe or frame_*.txt)
    #[arg(default_value = ".")]
    directory: PathBuf,

    /// Starting playback FPS
    #[arg(long, default_value_t = 24)]
    fps: u32,

    /// Play once instead of looping
    #[arg(long, default_value_t = false)]
    once: bool,
}

struct TerminalGuard {
    stdout: Stdout,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        let mut stdout = io::stdout();
        terminal::enable_raw_mode().context("enabling raw mode")?;
        execute!(stdout, EnterAlternateScreen, Hide, Clear(ClearType::All))
            .context("entering alternate screen")?;
        Ok(Self { stdout })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, ResetColor, Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let frames = load_frames(&args.directory)?;
    let has_any_color = frames.iter().any(Frame::has_color);

    let mut controller = AnimationController::new(args.fps);
    controller.set_frame_count(frames.len());
    if args.once {
        controller.set_loop_mode(LoopMode::Once);
    }
    controller.play();

    run_player(frames, has_any_color, controller)
}

fn run_player(
    frames: Vec<Frame>,
    has_any_color: bool,
    mut controller: AnimationController,
) -> Result<()> {
    if frames.is_empty() {
        bail!("No frames to display");
    }

    let mut terminal = TerminalGuard::enter()?;
    let mut needs_redraw = true;
    let mut last_tick = Instant::now();

    loop {
        if needs_redraw {
            let current_idx = controller.current_frame();
            let frame = frames
                .get(current_idx)
                .context("current frame index out of bounds")?;
            render_frame(
                &mut terminal.stdout,
                frame,
                &controller,
                current_idx,
                frames.len(),
                has_any_color,
            )?;
            terminal
                .stdout
                .flush()
                .context("flushing terminal output")?;
            needs_redraw = false;
        }

        let wait_timeout = if controller.is_playing() {
            let frame_duration = Duration::from_millis(controller.interval_ms() as u64);
            frame_duration.saturating_sub(last_tick.elapsed())
        } else {
            Duration::from_millis(250)
        };

        if event::poll(wait_timeout).context("polling terminal events")? {
            match event::read().context("reading terminal event")? {
                Event::Resize(_, _) => {
                    needs_redraw = true;
                }
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char(' ') => {
                            controller.toggle();
                            last_tick = Instant::now();
                            needs_redraw = true;
                        }
                        KeyCode::Right => {
                            controller.step_forward();
                            needs_redraw = true;
                        }
                        KeyCode::Left => {
                            controller.step_backward();
                            needs_redraw = true;
                        }
                        KeyCode::Home => {
                            controller.set_current_frame(0);
                            needs_redraw = true;
                        }
                        KeyCode::End => {
                            controller
                                .set_current_frame(controller.frame_count().saturating_sub(1));
                            needs_redraw = true;
                        }
                        KeyCode::Char('+') | KeyCode::Char('=') => {
                            controller.set_fps(controller.fps().saturating_add(1));
                            last_tick = Instant::now();
                            needs_redraw = true;
                        }
                        KeyCode::Char('-') | KeyCode::Char('_') => {
                            controller.set_fps(controller.fps().saturating_sub(1));
                            last_tick = Instant::now();
                            needs_redraw = true;
                        }
                        KeyCode::Char('l') => {
                            let next_mode = match controller.loop_mode() {
                                LoopMode::Loop => LoopMode::Once,
                                LoopMode::Once => LoopMode::Loop,
                            };
                            controller.set_loop_mode(next_mode);
                            needs_redraw = true;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        if controller.is_playing() {
            let frame_duration = Duration::from_millis(controller.interval_ms() as u64);
            if last_tick.elapsed() >= frame_duration {
                if controller.tick() {
                    needs_redraw = true;
                } else {
                    // Ensure status line updates when playback transitions to Finished.
                    needs_redraw = true;
                }
                last_tick = Instant::now();
            }
        }
    }

    Ok(())
}

fn render_frame(
    stdout: &mut Stdout,
    frame: &Frame,
    controller: &AnimationController,
    frame_index: usize,
    total_frames: usize,
    has_any_color: bool,
) -> Result<()> {
    let (term_width, term_height) = terminal::size().context("reading terminal size")?;
    let drawable_height = term_height.saturating_sub(1) as usize;
    let term_width_usize = term_width as usize;

    queue!(stdout, MoveTo(0, 0), Clear(ClearType::All)).context("clearing frame")?;

    if let Some(cframe) = frame.cframe.as_ref() {
        let frame_width = cframe.width as usize;
        let frame_height = cframe.height as usize;
        let draw_width = min(frame_width, term_width_usize);
        let draw_height = min(frame_height, drawable_height);

        let x_offset = term_width_usize.saturating_sub(draw_width) / 2;
        let y_offset = drawable_height.saturating_sub(draw_height) / 2;

        for row in 0..draw_height {
            let mut col = 0usize;
            while col < draw_width {
                if cframe.should_skip(row, col) {
                    col += 1;
                    continue;
                }

                let start_col = col;
                let (r, g, b) = cframe.rgb_at(row, col).unwrap_or((255, 255, 255));
                let mut run = String::new();
                run.push(cframe.char_at(row, col).unwrap_or(b' ') as char);
                col += 1;

                while col < draw_width {
                    if cframe.should_skip(row, col) {
                        break;
                    }

                    let next_color = cframe.rgb_at(row, col).unwrap_or((255, 255, 255));
                    if next_color != (r, g, b) {
                        break;
                    }

                    run.push(cframe.char_at(row, col).unwrap_or(b' ') as char);
                    col += 1;
                }

                queue!(
                    stdout,
                    MoveTo((x_offset + start_col) as u16, (y_offset + row) as u16),
                    SetForegroundColor(Color::Rgb { r, g, b }),
                    Print(&run)
                )
                .context("drawing colored run")?;
            }
        }
    } else {
        draw_text_frame(stdout, frame, term_width_usize, drawable_height)?;
    }

    draw_status_line(
        stdout,
        controller,
        frame_index,
        total_frames,
        has_any_color,
        term_width,
        term_height,
    )?;

    Ok(())
}

fn draw_text_frame(
    stdout: &mut Stdout,
    frame: &Frame,
    term_width: usize,
    drawable_height: usize,
) -> Result<()> {
    let lines: Vec<&str> = frame.content.lines().collect();
    let frame_height = lines.len();
    let frame_width = lines.iter().map(|line| line.len()).max().unwrap_or(0);
    let draw_width = min(frame_width, term_width);
    let draw_height = min(frame_height, drawable_height);

    let x_offset = term_width.saturating_sub(draw_width) / 2;
    let y_offset = drawable_height.saturating_sub(draw_height) / 2;

    for (row, line) in lines.iter().take(draw_height).enumerate() {
        let bytes = line.as_bytes();
        let row_width = min(bytes.len(), draw_width);
        let mut col = 0usize;

        while col < row_width {
            if bytes[col] == b' ' {
                col += 1;
                continue;
            }

            let start_col = col;
            while col < row_width && bytes[col] != b' ' {
                col += 1;
            }

            let text = std::str::from_utf8(&bytes[start_col..col]).unwrap_or("");
            queue!(
                stdout,
                MoveTo((x_offset + start_col) as u16, (y_offset + row) as u16),
                SetForegroundColor(Color::White),
                Print(text)
            )
            .context("drawing text run")?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_status_line(
    stdout: &mut Stdout,
    controller: &AnimationController,
    frame_index: usize,
    total_frames: usize,
    has_any_color: bool,
    term_width: u16,
    term_height: u16,
) -> Result<()> {
    let playback_state = format!("{:?}", controller.state()).to_lowercase();
    let loop_mode = match controller.loop_mode() {
        LoopMode::Loop => "loop",
        LoopMode::Once => "once",
    };
    let status = format!(
        "frame {}/{} | {} | {} fps | {} | color:{} | [space] play/pause [←/→] step [+/-] fps [l] loop [q] quit",
        frame_index + 1,
        total_frames,
        playback_state,
        controller.fps(),
        loop_mode,
        if has_any_color { "on" } else { "off" }
    );

    let status_line = truncate_to_width(&status, term_width as usize);
    let y = term_height.saturating_sub(1);
    let clear_line = " ".repeat(term_width as usize);

    queue!(
        stdout,
        MoveTo(0, y),
        SetForegroundColor(Color::DarkGrey),
        Print(clear_line),
        MoveTo(0, y),
        Print(status_line),
        ResetColor
    )
    .context("drawing status line")?;

    Ok(())
}

fn truncate_to_width(input: &str, width: usize) -> String {
    input.chars().take(width).collect()
}

fn load_frames(directory: &Path) -> Result<Vec<Frame>> {
    let cframe_paths = collect_frame_paths(directory, "cframe", false)?;
    if !cframe_paths.is_empty() {
        let mut frames = Vec::with_capacity(cframe_paths.len());
        for path in cframe_paths {
            let data = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            let cframe = parse_cframe(&data)
                .with_context(|| format!("parsing .cframe file {}", path.display()))?;
            let text = parse_cframe_text(&data)
                .with_context(|| format!("extracting text from {}", path.display()))?;
            frames.push(Frame::with_color(text, cframe));
        }
        return Ok(frames);
    }

    let txt_paths = collect_frame_paths(directory, "txt", true)?;
    if txt_paths.is_empty() {
        bail!(
            "No frame files found in {} (expected .cframe or frame_*.txt)",
            directory.display()
        );
    }

    let mut frames = Vec::with_capacity(txt_paths.len());
    for txt_path in txt_paths {
        let content = fs::read_to_string(&txt_path)
            .with_context(|| format!("reading {}", txt_path.display()))?;
        let content = normalize_frame_text(content);
        let cframe_path = txt_path.with_extension("cframe");

        if cframe_path.exists() {
            let data = fs::read(&cframe_path)
                .with_context(|| format!("reading {}", cframe_path.display()))?;
            let cframe = parse_cframe(&data)
                .with_context(|| format!("parsing .cframe file {}", cframe_path.display()))?;
            frames.push(Frame::with_color(content, cframe));
        } else {
            frames.push(Frame::text_only(content));
        }
    }

    Ok(frames)
}

fn collect_frame_paths(
    directory: &Path,
    extension: &str,
    require_frame_prefix: bool,
) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(directory)
        .with_context(|| format!("reading frame directory {}", directory.display()))?;

    let mut indexed = Vec::new();
    for (fallback, entry) in entries.enumerate() {
        let entry = entry.with_context(|| format!("reading entry in {}", directory.display()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if !ext.eq_ignore_ascii_case(extension) {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        if require_frame_prefix && !name.starts_with("frame_") {
            continue;
        }

        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        let index = FrameFile::extract_index(stem, fallback as u32);
        indexed.push((index, name, path));
    }

    indexed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    Ok(indexed.into_iter().map(|(_, _, path)| path).collect())
}

fn normalize_frame_text(mut text: String) -> String {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}
