use std::cmp::min;
use std::fs;
use std::io::{self, BufWriter, Stdout, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use cascii_core_view::{
    AnimationController, Frame, FrameColors, FrameFile, LoopMode, ProjectDetails, parse_cframe, parse_cframe_text,
    parse_packed_cframes,
};
use clap::Parser;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};

#[derive(Debug, Parser)]
#[command(author, version, about = "Minimal crossterm player for cascii .cframe/.txt frames")]
struct Args {
    /// Directory containing frame files (frame_*.cframe or frame_*.txt)
    #[arg(default_value = ".")]
    directory: PathBuf,

    /// Starting playback FPS (overrides details.toml)
    #[arg(long)]
    fps: Option<u32>,

    /// Play once instead of looping
    #[arg(long, default_value_t = false)]
    once: bool,

    /// Enable colored rendering from .cframe data
    #[arg(long, default_value_t = false)]
    color: bool,

    /// Packed multi-frame .cframe blob to play (relative paths use DIRECTORY)
    #[arg(long, value_name = "FILE")]
    packed: Option<PathBuf>,

    /// Start playback at this position (0.0 = first frame, 1.0 = last frame)
    #[arg(long, value_name = "POSITION")]
    start: Option<f64>,

    /// End playback at this position (0.0 = first frame, 1.0 = last frame)
    #[arg(long, value_name = "POSITION")]
    end: Option<f64>,

    /// Preserve native frame dimensions and crop content that exceeds the terminal
    #[arg(long, default_value_t = false)]
    no_fit: bool,
}

struct TerminalGuard {
    stdout: BufWriter<Stdout>,
}

struct DisplayState {
    colors: FrameColors,
    has_any_color: bool,
    has_audio: bool,
    fit_to_terminal: bool,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        let mut stdout = BufWriter::with_capacity(1024 * 1024, io::stdout());
        terminal::enable_raw_mode().context("enabling raw mode")?;
        execute!(stdout, EnterAlternateScreen, Hide, Clear(ClearType::All)).context("entering alternate screen")?;
        Ok(Self {stdout})
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
    let details = load_project_details(&args.directory);
    let colors = details.frame_colors();
    let has_audio = details.audio.unwrap_or(false);
    let fps = args.fps.unwrap_or_else(|| details.fps.unwrap_or(24));

    let frames = if let Some(packed_path) = args.packed.as_ref() {
        load_packed_frames(&resolve_packed_path(&args.directory, packed_path), args.color)?
    } else {
        load_frames(&args.directory, args.color)?
    };
    let has_any_color = frames.iter().any(Frame::has_color);

    let mut controller = AnimationController::new(fps);
    controller.set_frame_count(frames.len());
    if args.once {
        controller.set_loop_mode(LoopMode::Once);
    }
    configure_range(&mut controller, args.start, args.end)?;
    controller.play();

    let display = DisplayState {colors, has_any_color, has_audio, fit_to_terminal: !args.no_fit};

    run_player(frames, controller, display)
}

fn resolve_packed_path(directory: &Path, packed_path: &Path) -> PathBuf {
    if packed_path.is_absolute() {
        packed_path.to_path_buf()
    } else {
        directory.join(packed_path)
    }
}

fn configure_range(controller: &mut AnimationController, start: Option<f64>, end: Option<f64>) -> Result<()> {
    let start = start.unwrap_or(0.0);
    let end = end.unwrap_or(1.0);
    if !(0.0..=1.0).contains(&start) || !(0.0..=1.0).contains(&end) || start >= end {
        bail!("--start and --end must be between 0.0 and 1.0, with start less than end");
    }
    controller.set_range(start, end);
    controller.set_current_frame(controller.range_frames().0);
    Ok(())
}

fn run_player(frames: Vec<Frame>, mut controller: AnimationController, mut display: DisplayState) -> Result<()> {
    if frames.is_empty() {
        bail!("No frames to display");
    }

    let mut terminal = TerminalGuard::enter()?;
    let mut needs_redraw = true;
    let mut next_tick = Instant::now() + frame_duration(&controller);

    loop {
        if needs_redraw {
            let current_idx = controller.current_frame();
            let frame = frames.get(current_idx).context("current frame index out of bounds")?;
            render_frame(&mut terminal.stdout, frame, &controller, current_idx, frames.len(), &display)?;
            terminal.stdout.flush().context("flushing terminal output")?;
            needs_redraw = false;
        }

        let wait_timeout = if controller.is_playing() {
            next_tick.saturating_duration_since(Instant::now())
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
                            next_tick = Instant::now() + frame_duration(&controller);
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
                            let (range_start, _) = controller.range_frames();
                            controller.set_current_frame(range_start);
                            needs_redraw = true;
                        }
                        KeyCode::End => {
                            let (_, range_end) = controller.range_frames();
                            controller.set_current_frame(range_end);
                            needs_redraw = true;
                        }
                        KeyCode::Char('+') | KeyCode::Char('=') => {
                            controller.set_fps(controller.fps().saturating_add(1));
                            next_tick = Instant::now() + frame_duration(&controller);
                            needs_redraw = true;
                        }
                        KeyCode::Char('-') | KeyCode::Char('_') => {
                            controller.set_fps(controller.fps().saturating_sub(1));
                            next_tick = Instant::now() + frame_duration(&controller);
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
                        KeyCode::Char('f') => {
                            display.fit_to_terminal = !display.fit_to_terminal;
                            needs_redraw = true;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        if controller.is_playing() && Instant::now() >= next_tick {
            let now = Instant::now();
            let duration = frame_duration(&controller);
            let overdue_intervals = now.duration_since(next_tick).as_nanos() / duration.as_nanos();
            let ticks_due = (overdue_intervals + 1).min(u32::MAX as u128) as u32;
            advance_controller(&mut controller, ticks_due);
            next_tick += duration * ticks_due;
            needs_redraw = true;
        }
    }

    Ok(())
}

fn frame_duration(controller: &AnimationController) -> Duration {
    Duration::from_nanos((1_000_000_000_u64 / controller.fps() as u64).max(1))
}

fn advance_controller(controller: &mut AnimationController, ticks: u32) {
    if ticks == 0 || !controller.is_playing() {
        return;
    }

    let (range_start, range_end) = controller.range_frames();
    match controller.loop_mode() {
        LoopMode::Loop => {
            let range_len = range_end.saturating_sub(range_start) + 1;
            if range_len == 0 {
                return;
            }
            let current_offset = controller.current_frame().saturating_sub(range_start);
            let next_offset = (current_offset + ticks as usize % range_len) % range_len;
            controller.set_current_frame(range_start + next_offset);
        }
        LoopMode::Once => {
            let remaining = range_end.saturating_sub(controller.current_frame());
            if ticks as usize <= remaining {
                controller.set_current_frame(controller.current_frame() + ticks as usize);
            } else {
                controller.set_current_frame(range_end);
                controller.tick();
            }
        }
    }
}

fn render_frame(stdout: &mut BufWriter<Stdout>, frame: &Frame, controller: &AnimationController, frame_index: usize, total_frames: usize, display: &DisplayState) -> Result<()> {
    let terminal_size = terminal::size().context("reading terminal size")?;
    let (term_width, term_height) = terminal_size;
    let drawable_height = term_height.saturating_sub(1) as usize;
    let term_width_usize = term_width as usize;

    let (bg_r, bg_g, bg_b) = display.colors.background;
    queue!(stdout, MoveTo(0, 0), SetBackgroundColor(Color::Rgb {r: bg_r, g: bg_g, b: bg_b}), Clear(ClearType::All)).context("clearing frame")?;

    if let Some(cframe) = frame.cframe.as_ref() {
        let frame_width = cframe.width as usize;
        let frame_height = cframe.height as usize;
        let (draw_width, draw_height) = drawable_dimensions(frame_width, frame_height, term_width_usize, drawable_height, display.fit_to_terminal);

        let x_offset = term_width_usize.saturating_sub(draw_width) / 2;
        let y_offset = drawable_height.saturating_sub(draw_height) / 2;

        draw_cframe(stdout, cframe, draw_width, draw_height, x_offset, y_offset, display)?;
    } else {
        draw_text_frame(stdout, frame, term_width_usize, drawable_height, display)?;
    }

    draw_status_line(stdout, controller, frame_index, total_frames, terminal_size, display)?;
    Ok(())
}

fn draw_cframe(stdout: &mut BufWriter<Stdout>, cframe: &cascii_core_view::CFrameData, draw_width: usize, draw_height: usize, x_offset: usize, y_offset: usize, display: &DisplayState) -> Result<()> {
    for row in 0..draw_height {
        let source_row = source_index(row, draw_height, cframe.height as usize, display.fit_to_terminal);
        queue!(stdout, MoveTo(x_offset as u16, (y_offset + row) as u16)).context("moving to colored row")?;

        let mut col = 0usize;
        while col < draw_width {
            let source_col = source_index(col, draw_width, cframe.width as usize, display.fit_to_terminal);
            let background = cframe.bg_rgb_at(source_row, source_col).unwrap_or(display.colors.background);
            let foreground = cframe.has_visible_foreground(source_row, source_col).then(|| cframe.rgb_at(source_row, source_col).unwrap_or((255, 255, 255)));
            let mut run = String::new();
            run.push(if foreground.is_some() {cframe.char_at(source_row, source_col).unwrap_or(b' ') as char} else {' '});
            col += 1;

            while col < draw_width {
                let next_source_col = source_index(col, draw_width, cframe.width as usize, display.fit_to_terminal);
                let next_background = cframe.bg_rgb_at(source_row, next_source_col).unwrap_or(display.colors.background);
                let next_foreground = cframe.has_visible_foreground(source_row, next_source_col).then(|| cframe.rgb_at(source_row, next_source_col).unwrap_or((255, 255, 255)));
                if next_background != background || next_foreground != foreground {
                    break;
                }
                run.push(if next_foreground.is_some() {cframe.char_at(source_row, next_source_col).unwrap_or(b' ') as char} else {' '});
                col += 1;
            }

            let (bg_r, bg_g, bg_b) = background;
            queue!(stdout, SetBackgroundColor(Color::Rgb {r: bg_r, g: bg_g, b: bg_b})).context("setting colored run background")?;
            if let Some((fg_r, fg_g, fg_b)) = foreground {
                queue!(stdout, SetForegroundColor(Color::Rgb {r: fg_r, g: fg_g, b: fg_b})).context("setting colored run foreground")?;
            }
            queue!(stdout, Print(run)).context("drawing colored run")?;
        }
    }
    Ok(())
}

fn draw_text_frame(stdout: &mut BufWriter<Stdout>, frame: &Frame, term_width: usize, drawable_height: usize, display: &DisplayState) -> Result<()> {
    let lines: Vec<&str> = frame.content.lines().collect();
    let frame_height = lines.len();
    let frame_width = lines.iter().map(|line| line.chars().count()).max().unwrap_or(0);
    let (draw_width, draw_height) = drawable_dimensions(frame_width, frame_height, term_width, drawable_height, display.fit_to_terminal);

    let x_offset = term_width.saturating_sub(draw_width) / 2;
    let y_offset = drawable_height.saturating_sub(draw_height) / 2;

    for row in 0..draw_height {
        let source_row = source_index(row, draw_height, frame_height, display.fit_to_terminal);
        let line = lines.get(source_row).copied().unwrap_or("");
        let sampled_line = sample_text_line(line, frame_width, draw_width, display.fit_to_terminal);
        let mut col = 0usize;

        while col < draw_width {
            if sampled_line[col] == ' ' {
                col += 1;
                continue;
            }

            let start_col = col;
            let mut text = String::new();
            while col < draw_width {
                let sampled = sampled_line[col];
                if sampled == ' ' {
                    break;
                }
                text.push(sampled);
                col += 1;
            }

            let (fg_r, fg_g, fg_b) = display.colors.foreground;
            queue!(stdout, MoveTo((x_offset + start_col) as u16, (y_offset + row) as u16), SetForegroundColor(Color::Rgb {r: fg_r, g: fg_g, b: fg_b}), Print(text))
            .context("drawing text run")?;
        }
    }

    Ok(())
}

fn sample_text_line(line: &str, source_width: usize, draw_width: usize, fit_to_terminal: bool) -> Vec<char> {
    let characters: Vec<char> = line.chars().collect();
    (0..draw_width)
        .map(|col| {
            let source_col = source_index(col, draw_width, source_width, fit_to_terminal);
            characters.get(source_col).copied().unwrap_or(' ')
        }).collect()
}

fn drawable_dimensions(frame_width: usize, frame_height: usize, max_width: usize, max_height: usize, fit_to_terminal: bool) -> (usize, usize) {
    if frame_width == 0 || frame_height == 0 || max_width == 0 || max_height == 0 {
        return (0, 0);
    }
    if !fit_to_terminal {
        return (min(frame_width, max_width), min(frame_height, max_height));
    }

    let scale = (max_width as f64 / frame_width as f64).min(max_height as f64 / frame_height as f64).min(1.0);
    let width = (frame_width as f64 * scale).floor().max(1.0) as usize;
    let height = (frame_height as f64 * scale).floor().max(1.0) as usize;
    (width, height)
}

fn source_index(target_index: usize, target_len: usize, source_len: usize, fit_to_terminal: bool) -> usize {
    debug_assert!(target_index < target_len);
    debug_assert!(target_len > 0);
    debug_assert!(source_len > 0);
    if !fit_to_terminal {
        return target_index;
    }

    let numerator = (target_index as u128 * 2 + 1) * source_len as u128;
    let denominator = target_len as u128 * 2;
    (numerator / denominator) as usize
}

fn draw_status_line(stdout: &mut BufWriter<Stdout>, controller: &AnimationController, frame_index: usize, total_frames: usize, terminal_size: (u16, u16), display: &DisplayState) -> Result<()> {
    let (term_width, term_height) = terminal_size;
    let playback_state = format!("{:?}", controller.state()).to_lowercase();
    let loop_mode = match controller.loop_mode() {
        LoopMode::Loop => "loop",
        LoopMode::Once => "once",
    };
    let (range_start, range_end) = controller.range_frames();
    let status = format!("frame {}/{} | range {}-{} | {} | {} fps | {} | fit:{} | color:{} | audio:{} | [space] play/pause [←/→] step [home/end] range [+/-] fps [f] fit [l] loop [q] quit", frame_index + 1, total_frames, range_start + 1, range_end + 1, playback_state, controller.fps(), loop_mode, if display.fit_to_terminal {"on"} else {"off"}, if display.has_any_color {"on"} else {"off"}, if display.has_audio {"yes"} else {"no"});
    let status_line = truncate_to_width(&status, term_width as usize);
    let y = term_height.saturating_sub(1);
    let clear_line = " ".repeat(term_width as usize);
    queue!(stdout, MoveTo(0, y), SetForegroundColor(Color::DarkGrey), SetBackgroundColor(Color::Reset), Print(clear_line), MoveTo(0, y), Print(status_line), ResetColor).context("drawing status line")?;
    Ok(())
}

fn truncate_to_width(input: &str, width: usize) -> String {
    input.chars().take(width).collect()
}

fn load_project_details(directory: &Path) -> ProjectDetails {
    let toml_path = directory.join("details.toml");
    if let Ok(content) = fs::read_to_string(&toml_path) {
        ProjectDetails::from_toml_str(&content).unwrap_or_default()
    } else {
        ProjectDetails::default()
    }
}

fn load_frames(directory: &Path, use_color: bool) -> Result<Vec<Frame>> {
    // Prefer .txt files when available (monochrome by default).
    let txt_paths = collect_frame_paths(directory, "txt", true)?;
    if !txt_paths.is_empty() {
        let mut frames = Vec::with_capacity(txt_paths.len());
        for txt_path in &txt_paths {
            let content = fs::read_to_string(txt_path).with_context(|| format!("reading {}", txt_path.display()))?;
            let content = normalize_frame_text(content);
            let cframe_path = txt_path.with_extension("cframe");

            if use_color && cframe_path.exists() {
                let data = fs::read(&cframe_path).with_context(|| format!("reading {}", cframe_path.display()))?;
                let cframe = parse_cframe(&data).with_context(|| format!("parsing .cframe file {}", cframe_path.display()))?;
                frames.push(Frame::with_color(content, cframe));
            } else {
                frames.push(Frame::text_only(content));
            }
        }
        return Ok(frames);
    }

    // Fallback: load .cframe files directly when no .txt files exist.
    let cframe_paths = collect_frame_paths(directory, "cframe", false)?;
    if cframe_paths.is_empty() {
        bail!("No frame files found in {} (expected .cframe or frame_*.txt)", directory.display());
    }

    let mut frames = Vec::with_capacity(cframe_paths.len());
    for path in cframe_paths {
        let data = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let cframe = parse_cframe(&data)
            .with_context(|| format!("parsing .cframe file {}", path.display()))?;
        let text = parse_cframe_text(&data)
            .with_context(|| format!("extracting text from {}", path.display()))?;
        if use_color {
            frames.push(Frame::with_color(text, cframe));
        } else {
            frames.push(Frame::text_only(text));
        }
    }

    Ok(frames)
}

fn load_packed_frames(path: &Path, use_color: bool) -> Result<Vec<Frame>> {
    let data = fs::read(path).with_context(|| format!("reading packed animation {}", path.display()))?;
    frames_from_packed_data(&data, use_color).with_context(|| format!("parsing packed animation {}", path.display()))
}

fn frames_from_packed_data(data: &[u8], use_color: bool) -> Result<Vec<Frame>> {
    let blob = parse_packed_cframes(data)?;
    let mut frames = Vec::with_capacity(blob.len());
    for index in 0..blob.len() {
        let cframe = blob.decode_frame(index).with_context(|| format!("decoding packed animation frame {index}"))?;
        let text = cframe.to_text();
        if use_color {
            frames.push(Frame::with_color(text, cframe));
        } else {
            frames.push(Frame::text_only(text));
        }
    }
    Ok(frames)
}

fn collect_frame_paths(directory: &Path, extension: &str, require_frame_prefix: bool) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(directory).with_context(|| format!("reading frame directory {}", directory.display()))?;
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

        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_string();
        if require_frame_prefix && !name.starts_with("frame_") {
            continue;
        }

        let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configures_a_playback_range_and_starts_at_its_first_frame() {
        let mut controller = AnimationController::new(24);
        controller.set_frame_count(100);

        configure_range(&mut controller, Some(0.25), Some(0.75)).unwrap();

        assert_eq!(controller.range_frames(), (25, 74));
        assert_eq!(controller.current_frame(), 25);
    }

    #[test]
    fn rejects_invalid_playback_ranges() {
        let mut controller = AnimationController::new(24);
        controller.set_frame_count(2);

        assert!(configure_range(&mut controller, Some(0.75), Some(0.25)).is_err());
        assert!(configure_range(&mut controller, Some(-0.1), Some(0.5)).is_err());
        assert!(configure_range(&mut controller, Some(0.1), Some(1.1)).is_err());
    }

    #[test]
    fn resolves_relative_packed_paths_against_the_frame_directory() {
        assert_eq!(resolve_packed_path(Path::new("frames"), Path::new("animation.cframes")), PathBuf::from("frames/animation.cframes"));
    }

    #[test]
    fn scales_large_frames_to_fit_without_upscaling() {
        assert_eq!(drawable_dimensions(300, 300, 120, 40, true), (40, 40));
        assert_eq!(drawable_dimensions(80, 24, 120, 40, true), (80, 24));
    }

    #[test]
    fn native_mode_crops_instead_of_scaling() {
        assert_eq!(drawable_dimensions(300, 300, 120, 40, false), (120, 40));
    }

    #[test]
    fn maps_scaled_cells_back_to_the_source_grid() {
        assert_eq!(source_index(0, 40, 300, true), 3);
        assert_eq!(source_index(20, 40, 300, true), 153);
        assert_eq!(source_index(39, 40, 300, true), 296);
    }

    #[test]
    fn native_mode_maps_directly_to_source_cells() {
        assert_eq!(source_index(0, 40, 300, false), 0);
        assert_eq!(source_index(20, 40, 300, false), 20);
        assert_eq!(source_index(39, 40, 300, false), 39);
    }

    #[test]
    fn catches_up_looping_playback_without_slowing_the_timeline() {
        let mut controller = AnimationController::new(30);
        controller.set_frame_count(10);
        controller.play();

        advance_controller(&mut controller, 12);

        assert_eq!(controller.current_frame(), 2);
        assert!(controller.is_playing());
    }

    #[test]
    fn finishes_once_playback_when_catch_up_passes_the_last_frame() {
        let mut controller = AnimationController::new(30);
        controller.set_frame_count(10);
        controller.set_loop_mode(LoopMode::Once);
        controller.play();

        advance_controller(&mut controller, 12);

        assert_eq!(controller.current_frame(), 9);
        assert!(!controller.is_playing());
    }

    #[test]
    fn text_sampling_preserves_unicode_characters() {
        assert_eq!(sample_text_line("é λ", 3, 3, false), vec!['é', ' ', 'λ']);
    }

    #[test]
    fn decodes_packed_frames_with_cell_backgrounds() {
        let mut data = Vec::new();
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&2_u32.to_le_bytes());
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&[b'A', 255, 0, 0, b'B', 0, 255, 0]);
        data.push(cascii_core_view::CFRAME_EXT_FLAG_HAS_BG);
        data.extend_from_slice(&[10, 20, 30, 40, 50, 60]);

        let frames = frames_from_packed_data(&data, true).unwrap();

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].content, "AB\n");
        let cframe = frames[0].cframe.as_ref().unwrap();
        assert_eq!(cframe.bg_rgb_at(0, 0), Some((10, 20, 30)));
        assert_eq!(cframe.bg_rgb_at(0, 1), Some((40, 50, 60)));
    }

    #[test]
    fn packed_frames_can_use_the_text_fallback() {
        let mut data = Vec::new();
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&[b'X', 255, 255, 255]);

        let frames = frames_from_packed_data(&data, false).unwrap();

        assert_eq!(frames[0].content, "X\n");
        assert!(frames[0].cframe.is_none());
    }
}
