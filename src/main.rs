use std::cmp::min;
use std::fs;
use std::io::{self, BufWriter, Stdout, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use cascii_core_view::{AnimationController, Frame, FrameColors, FrameFile, LoopMode, ProjectDetails, AnimationState};
use clap::{Args as ClapArgs, Parser, Subcommand, ValueEnum};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};

mod audio;
mod detail;
mod frames;
mod input;
mod tiles;
mod library;
mod settings;

use audio::Audio;
use detail::{Curve, Detail, Direction};
use frames::Frames;
use input::Input;

#[derive(Debug, Parser)]
#[command(author, version, about = "Terminal player for fixed and responsive Cascii animations")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    playback: PlaybackArgs,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Play frames from a path or from a saved animation name
    Play(PlaybackArgs),

    /// Save a frame directory or packed binary for later playback
    Save {
        /// File or directory containing the animation frames
        source: PathBuf,

        /// Name used to save and play the animation (defaults to the source name)
        name: Option<String>,
    },

    /// Show or change persistent settings
    Config(ConfigArgs),
}

#[derive(Debug, ClapArgs)]
struct ConfigArgs {
    #[command(subcommand)]
    command: Option<ConfigCommand>,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Create settings.json if it does not exist
    Init,

    /// Print all settings as JSON
    Show,

    /// Print the path to settings.json
    Path,

    /// Change a setting
    Set {
        #[command(subcommand)]
        setting: ConfigSetting,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigSetting {
    /// Set the directory where named animations are saved
    SavePath {
        path: PathBuf,
    },

    /// Enable or disable colored rendering by default
    Color {
        value: OnOff,
    },

    /// Set the default playback speed in frames per second
    Fps {
        /// Positive whole-number frame rate
        #[arg(value_parser = parse_positive_fps)]
        fps: u32,
    },

    /// Enable or disable fit-to-terminal rendering by default
    Fit {
        value: OnOff,
    },

    /// Enable or disable looping by default
    Loop {
        value: OnOff,
    },

    /// Hide or show the entire information bar by default
    HideBar {
        value: OnOff,
    },

    /// Show or hide one field in the information bar
    Bar {
        field: BarField,
        value: OnOff,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OnOff {
    #[value(alias = "true", alias = "yes")]
    On,
    #[value(alias = "false", alias = "no")]
    Off,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BarField {
    Frame,
    Range,
    State,
    Fps,
    Loop,
    Fit,
    Color,
    Audio,
    Controls,
}

#[derive(Debug, ClapArgs)]
struct PlaybackArgs {
    /// Frame directory, animation file, HTTP(S) asset URL, or saved animation name
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Playback FPS (overrides the manifest, details.toml and settings)
    #[arg(long, value_parser = parse_positive_fps)]
    fps: Option<u32>,

    /// Play once instead of looping
    #[arg(long, default_value_t = false, conflicts_with = "looping")]
    once: bool,

    /// Loop this playback
    #[arg(long = "loop", default_value_t = false, conflicts_with = "once")]
    looping: bool,

    /// Enable colored rendering from .cframe data
    #[arg(long, default_value_t = false, conflicts_with = "no_color")]
    color: bool,

    /// Disable colored rendering for this playback
    #[arg(long, default_value_t = false, conflicts_with = "color")]
    no_color: bool,

    /// Packed multi-frame blob to play (relative paths use PATH)
    #[arg(long, value_name = "FILE")]
    packed: Option<PathBuf>,

    /// Start playback at this position (0.0 = first frame, 1.0 = last frame)
    #[arg(long, value_name = "POSITION")]
    start: Option<f64>,

    /// End playback at this position (0.0 = first frame, 1.0 = last frame)
    #[arg(long, value_name = "POSITION")]
    end: Option<f64>,

    /// Fit frames within the terminal for this playback
    #[arg(long, default_value_t = false, conflicts_with = "no_fit")]
    fit: bool,

    /// Preserve native frame dimensions and crop content that exceeds the terminal
    #[arg(long, default_value_t = false, conflicts_with = "fit")]
    no_fit: bool,

    /// Select the nearest generated grid width
    #[arg(long, conflicts_with_all = ["column_map", "transition"], value_parser = clap::value_parser!(u32).range(1..))]
    columns: Option<u32>,

    /// Follow the authored width for each frame
    #[arg(long, conflicts_with = "transition")]
    column_map: bool,

    /// Animate detail across the timeline
    #[arg(long, value_enum)]
    transition: Option<Curve>,

    /// Direction of the detail transition (default: down)
    #[arg(long, value_enum, requires = "transition")]
    direction: Option<Direction>,

    /// Start paused on the first frame of the active range
    #[arg(long)]
    paused: bool,

    /// Play without opening an audio device
    #[arg(long)]
    no_audio: bool,

    /// Show the information bar for this playback
    #[arg(long, default_value_t = false, conflicts_with = "hide_bar")]
    show_bar: bool,

    /// Hide the information bar for this playback
    #[arg(long, default_value_t = false, conflicts_with = "show_bar")]
    hide_bar: bool,
}

struct TerminalGuard {
    stdout: BufWriter<Stdout>,
}

#[derive(Clone, Copy, PartialEq)]
enum ColorMode { Full, Project, Mono }

struct DisplayState {
    mode: ColorMode,
    grid: Option<(u32, u32, Detail)>,
    seek_input: Option<String>,
    audio_status: String,
    colors: FrameColors,
    has_any_color: bool,
    fit_to_terminal: bool,
    hide_bar: bool,
    bar: settings::BarSettings,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        let stdout = BufWriter::with_capacity(1024 * 1024, io::stdout());
        terminal::enable_raw_mode().context("enabling raw mode")?;
        let mut guard = Self {stdout};
        execute!(guard.stdout, EnterAlternateScreen, Hide, Clear(ClearType::All)).context("entering alternate screen")?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, ResetColor, Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let app_paths = settings::app_paths()?;
    let mut app_settings = settings::load_or_create(&app_paths)?;

    match cli.command {
        Some(Command::Play(args)) => play(args, &app_settings),
        Some(Command::Save {source, name}) => {
            let (name, destination) = save_animation(&source, name, &app_settings.save_path)?;
            println!("Saved '{name}' to {}", destination.display());
            println!("Play it with: {} play {name}", settings::APP_NAME);
            Ok(())
        }
        Some(Command::Config(args)) => run_config(args, &app_paths, &mut app_settings),
        None => play(cli.playback, &app_settings),
    }
}

fn save_animation(source: &Path, requested_name: Option<String>, save_path: &Path) -> Result<(String, PathBuf)> {
    let input = Input::from_path(source)?;
    let name = match requested_name {
        Some(name) => name,
        None if source.is_dir() => source.canonicalize()?.file_name().and_then(|s| s.to_str()).context("Cannot infer animation name")?.to_string(),
        None => {
            let file = input.name().context("Provide a name for this animation")?;
            Path::new(file.strip_suffix(".gz").unwrap_or(file)).file_stem().and_then(|s| s.to_str()).context("Cannot infer animation name")?.to_string()
        }
    };
    if source.is_dir() {
        validate_animation_source(source)?;
        return Ok((name.clone(), library::save(source, &name, save_path)?));
    }

    // Build and validate a self-contained package before making a named entry visible.
    let stage = tempfile::tempdir()?;
    let original = input.name().context("Missing input filename")?;
    let filename = if original.ends_with(".txt") {"frame_0001.txt"} else {original};
    let file = stage.path().join(filename);
    input.copy_to(&file)?;
    let mut frames = Frames::open(&Input::Path(file.clone()))?;
    if let Frames::Tiles(tiles) = &frames {
        tiles.copy_sidecars_from(&input, stage.path())?;
    }
    if let Input::Path(path) = &input {
        let details = path.with_file_name("details.toml");
        if details.is_file() { Input::Path(details).copy_to(&stage.path().join("details.toml"))?; }
    }
    if let Some(audio) = audio_input(&input, frames.manifest().is_some_and(|m| m.audio.is_some()))? {
        audio.copy_to(&stage.path().join("audio.mp3"))?;
    }
    frames.validate()?;
    drop(frames);
    // A packed file can legally have an arbitrary extension. Give saved copies a
    // discoverable name so named playback goes through the same format dispatch.
    if !is_playback_filename(filename) && !filename.ends_with(".txt") && !filename.ends_with(".cframe") {
        fs::rename(&file, stage.path().join("animation.bin"))?;
    }
    let destination = library::save(stage.path(), &name, save_path)?;
    Ok((name, destination))
}

fn validate_animation_source(source: &Path) -> Result<()> {
    Frames::open(&Input::from_path(source)?)?.validate()
}

fn audio_input(input: &Input, declared: bool) -> Result<Option<Input>> {
    let audio = match input {
        Input::Path(path) if path.is_dir() => Input::Path(path.join("audio.mp3")),
        _ => input.sibling("audio.mp3")?,
    };
    Ok(match &audio {
        Input::Path(path) if path.is_file() => Some(audio),
        Input::Url(_, _) if declared => Some(audio),
        _ => None,
    })
}

fn run_config(args: ConfigArgs, app_paths: &settings::AppPaths, app_settings: &mut settings::Settings) -> Result<()> {
    match args.command.unwrap_or(ConfigCommand::Show) {
        ConfigCommand::Init => {
            println!("Settings initialized at {}", app_paths.settings_file.display());
        }
        ConfigCommand::Show => {
            println!("{}", serde_json::to_string_pretty(app_settings).context("serializing settings")?);
        }
        ConfigCommand::Path => {
            println!("{}", app_paths.settings_file.display());
        }
        ConfigCommand::Set {setting} => match setting {
            ConfigSetting::SavePath {path} => {
                settings::set_save_path(app_paths, app_settings, &path)?;
                println!("Save path set to {}", app_settings.save_path.display());
            }
            ConfigSetting::Color {value} => {
                let enabled = matches!(value, OnOff::On);
                settings::set_color(app_paths, app_settings, enabled)?;
                println!("Default color rendering set to {}", if enabled {"on"} else {"off"});
            }
            ConfigSetting::Fps {fps} => {
                settings::set_fps(app_paths, app_settings, fps)?;
                println!("Default playback speed set to {fps} FPS");
            }
            ConfigSetting::Fit {value} => {
                let enabled = matches!(value, OnOff::On);
                settings::set_fit(app_paths, app_settings, enabled)?;
                println!("Default fit-to-terminal rendering set to {}", if enabled {"on"} else {"off"});
            }
            ConfigSetting::Loop {value} => {
                let enabled = matches!(value, OnOff::On);
                settings::set_loop(app_paths, app_settings, enabled)?;
                println!("Default looping set to {}", if enabled {"on"} else {"off"});
            }
            ConfigSetting::HideBar {value} => {
                let hidden = matches!(value, OnOff::On);
                settings::set_hide_bar(app_paths, app_settings, hidden)?;
                println!("Default information bar hiding set to {}", if hidden {"on"} else {"off"});
            }
            ConfigSetting::Bar {field, value} => {
                let visible = matches!(value, OnOff::On);
                set_bar_field(&mut app_settings.bar, field, visible);
                settings::write(app_paths, app_settings)?;
                println!("Information bar field {} set to {}", field_name(field), if visible {"visible"} else {"hidden"});
            }
        }
    }
    Ok(())
}

fn play(mut args: PlaybackArgs, app_settings: &settings::Settings) -> Result<()> {
    let use_color = playback_uses_color(&args, app_settings);
    let fit_to_terminal = playback_fits_terminal(&args, app_settings);
    let loop_playback = playback_loops(&args, app_settings);
    let hide_bar = playback_hides_bar(&args, app_settings);
    args.path = library::resolve(&args.path, &app_settings.save_path);
    let input = if let Some(packed) = &args.packed {
        if !args.path.is_dir() { bail!("The playback path must be a directory when --packed is used"); }
        Input::from_path(&resolve_packed_path(&args.path, packed))?
    } else {
        Input::from_path(&args.path)?
    };
    let frames = Frames::open(&input)?;
    let manifest = frames.manifest();
    let project = if args.packed.is_some() { Input::Path(args.path.clone()) } else { input.clone() };
    let details = match &project {
        Input::Path(path) => load_project_details(if path.is_dir() {path} else {path.parent().unwrap_or(Path::new("."))}),
        Input::Url(_, _) => ProjectDetails::default(),
    };
    let project_fps = manifest.map(|m| m.fps).or(details.fps).filter(|fps| *fps > 0);
    let fps = playback_fps(args.fps, project_fps, app_settings);
    let explicit_detail = args.columns.is_some() || args.column_map || args.transition.is_some();
    if explicit_detail && manifest.is_none() { bail!("Detail selection requires a multi-size animation"); }
    let detail = match (args.columns, args.transition) {
        (Some(columns), _) => Detail::Width(columns),
        (_, Some(curve)) => Detail::Transition(curve, args.direction.unwrap_or(Direction::Down)),
        _ if args.column_map || !fit_to_terminal => Detail::Authored,
        _ => Detail::Auto,
    };
    let mut controller = AnimationController::new(fps);
    controller.set_frame_count(frames.count());
    if !loop_playback { controller.set_loop_mode(LoopMode::Once); }
    configure_range(&mut controller, args.start, args.end)?;
    if !args.paused { controller.play(); }

    let track = audio_input(&project, manifest.is_some_and(|m| m.audio.is_some()) || details.audio == Some(true))?;
    let mut audio_status = if track.is_some() {"disabled"} else {"no"}.to_string();
    let audio = if let Some(track) = track.filter(|_| !args.no_audio) {
        match Audio::open(track, project_fps.unwrap_or(app_settings.fps)) {
            Ok(audio) => { audio_status = audio.level(); Some(audio) }
            Err(e) => { eprintln!("Audio unavailable: {e:#}"); audio_status = "unavailable".into(); None }
        }
    } else { None };
    let has_any_color = frames.has_color();
    let display = DisplayState {
        mode: if use_color && has_any_color {ColorMode::Full} else {ColorMode::Project},
        colors: details.frame_colors(), has_any_color, audio_status, grid: None, seek_input: None,
        fit_to_terminal, hide_bar, bar: app_settings.bar.clone(),
    };
    run_player(frames, controller, display, detail, audio)
}

fn playback_uses_color(args: &PlaybackArgs, settings: &settings::Settings) -> bool {
    if args.color {
        true
    } else if args.no_color {
        false
    } else {
        settings.color
    }
}

fn playback_fps(requested_fps: Option<u32>, project_fps: Option<u32>, settings: &settings::Settings) -> u32 {
    requested_fps.or(project_fps).unwrap_or(settings.fps)
}

fn playback_fits_terminal(args: &PlaybackArgs, settings: &settings::Settings) -> bool {
    if args.fit {
        true
    } else if args.no_fit {
        false
    } else {
        settings.fit
    }
}

fn playback_loops(args: &PlaybackArgs, settings: &settings::Settings) -> bool {
    if args.looping {
        true
    } else if args.once {
        false
    } else {
        settings.loop_playback
    }
}

fn playback_hides_bar(args: &PlaybackArgs, settings: &settings::Settings) -> bool {
    if args.hide_bar {
        true
    } else if args.show_bar {
        false
    } else {
        settings.hide_bar
    }
}

fn set_bar_field(bar: &mut settings::BarSettings, field: BarField, visible: bool) {
    match field {
        BarField::Frame => bar.frame = visible,
        BarField::Range => bar.range = visible,
        BarField::State => bar.state = visible,
        BarField::Fps => bar.fps = visible,
        BarField::Loop => bar.loop_mode = visible,
        BarField::Fit => bar.fit = visible,
        BarField::Color => bar.color = visible,
        BarField::Audio => bar.audio = visible,
        BarField::Controls => bar.controls = visible,
    }
}

fn field_name(field: BarField) -> &'static str {
    match field {
        BarField::Frame => "frame",
        BarField::Range => "range",
        BarField::State => "state",
        BarField::Fps => "fps",
        BarField::Loop => "loop",
        BarField::Fit => "fit",
        BarField::Color => "color",
        BarField::Audio => "audio",
        BarField::Controls => "controls",
    }
}

fn parse_positive_fps(value: &str) -> std::result::Result<u32, String> {
    let fps = value.parse::<u32>().map_err(|_| "FPS must be a positive whole number".to_string())?;
    if fps == 0 {
        return Err("FPS must be at least 1".to_string());
    }
    Ok(fps)
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

fn run_player(mut frames: Frames, mut controller: AnimationController, mut display: DisplayState, mut detail: Detail, mut audio: Option<Audio>) -> Result<()> {
    let mut terminal = TerminalGuard::enter()?;
    // Register resize notifications before drawing the first (possibly paused) frame.
    event::poll(Duration::ZERO)?;
    let mut needs_redraw = true;
    let mut sync_audio = true;
    let mut next_tick = Instant::now() + frame_duration(&controller);
    let mut clock_before_seek = None;
    let mut reset_clock = true;
    let project_colors = display.colors.clone();
    let mut cached = None;

    loop {
        if needs_redraw {
            let size = terminal::size().context("reading terminal size")?;
            let rows = size.1.saturating_sub(u16::from(!display.hide_bar || display.seek_input.is_some()));
            let index = controller.current_frame();
            let columns = frames.manifest().map_or(0, |m| detail.columns(m, index, size.0, rows));
            let key = (index, columns, display.mode == ColorMode::Full);
            if cached.as_ref().is_none_or(|(last, _)| *last != key) {
                let frame = frames.frame(index, columns, key.2).with_context(|| format!("loading frame {}", index + 1))?;
                cached = Some((key, frame));
            }
            let frame = &cached.as_ref().unwrap().1;
            display.grid = (columns > 0).then(|| (columns, frame.dimensions().1 as u32, detail));
            render_frame(&mut terminal.stdout, frame, &controller, index, frames.count(), &display, size)?;
            terminal.stdout.flush().context("flushing terminal output")?;
            needs_redraw = false;
        }
        if sync_audio {
            if let Some(track) = &mut audio && let Err(e) = track.sync(&controller) {
                display.audio_status = format!("unavailable ({e})");
                audio = None;
                needs_redraw = true;
            }
            sync_audio = false;
        }
        if reset_clock {
            // A network fetch or decoder seek can delay the first visible frame.
            next_tick = Instant::now() + frame_duration(&controller);
            reset_clock = false;
        }

        let wait = if controller.is_playing() { next_tick.saturating_duration_since(Instant::now()) } else { Duration::from_millis(250) };
        if event::poll(wait)? {
            match event::read()? {
                Event::Resize(_, _) => needs_redraw = true,
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    needs_redraw = true;
                    if let Some(entry) = &mut display.seek_input {
                        match key.code {
                            KeyCode::Char(c) if c.is_ascii_digit() && entry.len() < 12 => entry.push(c),
                            KeyCode::Backspace => { entry.pop(); }
                            KeyCode::Esc | KeyCode::Enter => {
                                controller = clock_before_seek.take().expect("seek prompt saves the controller");
                                if key.code == KeyCode::Enter && let Ok(target) = entry.parse::<usize>() {
                                    if controller.state() == AnimationState::Finished { controller.stop(); }
                                    controller.set_current_frame(target.saturating_sub(1));
                                }
                                display.seek_input = None;
                                sync_audio = true;
                            }
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => break,
                            KeyCode::Char(' ') => { controller.toggle(); sync_audio = true; }
                            KeyCode::Right => { controller.step_forward(); sync_audio = true; }
                            KeyCode::Left => { controller.step_backward(); sync_audio = true; }
                            KeyCode::Home | KeyCode::End => {
                                let (first, last) = controller.range_frames();
                                controller.set_current_frame(if key.code == KeyCode::Home {first} else {last});
                                sync_audio = true;
                            }
                            KeyCode::Char('g') => {
                                clock_before_seek = Some(controller.clone());
                                controller.pause();
                                display.seek_input = Some(String::new());
                                sync_audio = true;
                            }
                            KeyCode::Char('+' | '=') => { controller.set_fps(controller.fps().saturating_add(1)); sync_audio = true; }
                            KeyCode::Char('-' | '_') => { controller.set_fps(controller.fps().saturating_sub(1)); sync_audio = true; }
                            KeyCode::Char('l') => {
                                controller.set_loop_mode(match controller.loop_mode() {LoopMode::Loop => LoopMode::Once, LoopMode::Once => LoopMode::Loop});
                            }
                            KeyCode::Char('f') => {
                                display.fit_to_terminal = !display.fit_to_terminal;
                                detail = if display.fit_to_terminal {Detail::Auto} else {Detail::Authored};
                            }
                            KeyCode::Char('a') => { detail = Detail::Auto; display.fit_to_terminal = true; }
                            KeyCode::Char('p') => detail = Detail::Authored,
                            KeyCode::Char('t') if frames.manifest().is_some() => detail = detail.next_curve(),
                            KeyCode::Char('r') => detail = detail.reverse(),
                            KeyCode::Char('[' | ']') => {
                                if let (Some(manifest), Some((columns, _, _))) = (frames.manifest(), display.grid) {
                                    detail.step(manifest, columns, key.code == KeyCode::Char(']'));
                                }
                            }
                            KeyCode::Char('c') => {
                                display.mode = match display.mode {
                                    ColorMode::Full => ColorMode::Mono,
                                    ColorMode::Mono if project_colors != ProjectDetails::default().frame_colors() || !display.has_any_color => ColorMode::Project,
                                    ColorMode::Mono | ColorMode::Project if display.has_any_color => ColorMode::Full,
                                    _ => ColorMode::Mono,
                                };
                                display.colors = if display.mode == ColorMode::Mono {ProjectDetails::default().frame_colors()} else {project_colors.clone()};
                            }
                            KeyCode::Char('m' | ',' | '.') => {
                                if let Some(track) = &mut audio {
                                    match key.code {
                                        KeyCode::Char('m') => track.toggle_mute(),
                                        KeyCode::Char(',') => track.adjust_volume(-0.1),
                                        _ => track.adjust_volume(0.1),
                                    }
                                    display.audio_status = track.level();
                                }
                            }
                            _ => {}
                        }
                    }
                    if sync_audio { reset_clock = true; }
                }
                _ => {}
            }
        }

        if !reset_clock && controller.is_playing() && Instant::now() >= next_tick {
            let duration = frame_duration(&controller);
            let ticks_due = (1 + Instant::now().duration_since(next_tick).as_nanos() / duration.as_nanos()).min(u32::MAX as u128) as u32;
            let crossed_end = u64::from(ticks_due) > (controller.range_frames().1 - controller.current_frame()) as u64;
            advance_controller(&mut controller, ticks_due);
            sync_audio |= crossed_end;
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

fn render_frame(stdout: &mut BufWriter<Stdout>, frame: &Frame, controller: &AnimationController, frame_index: usize, total_frames: usize, display: &DisplayState, terminal_size: (u16, u16)) -> Result<()> {
    let (term_width, term_height) = terminal_size;
    let bar_height = if display.hide_bar && display.seek_input.is_none() {0} else {1};
    let drawable_height = term_height.saturating_sub(bar_height) as usize;
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

    if !display.hide_bar || display.seek_input.is_some() {
        draw_status_line(stdout, controller, frame_index, total_frames, terminal_size, display)?;
    }
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
            run.push(if foreground.is_some() {printable(cframe.char_at(source_row, source_col).unwrap_or(b' ') as char)} else {' '});
            col += 1;

            while col < draw_width {
                let next_source_col = source_index(col, draw_width, cframe.width as usize, display.fit_to_terminal);
                let next_background = cframe.bg_rgb_at(source_row, next_source_col).unwrap_or(display.colors.background);
                let next_foreground = cframe.has_visible_foreground(source_row, next_source_col).then(|| cframe.rgb_at(source_row, next_source_col).unwrap_or((255, 255, 255)));
                if next_background != background || next_foreground != foreground {
                    break;
                }
                run.push(if next_foreground.is_some() {printable(cframe.char_at(source_row, next_source_col).unwrap_or(b' ') as char)} else {' '});
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

fn printable(character: char) -> char {
    if character.is_control() { ' ' } else { character }
}

fn sample_text_line(line: &str, source_width: usize, draw_width: usize, fit_to_terminal: bool) -> Vec<char> {
    let characters: Vec<char> = line.chars().collect();
    (0..draw_width).map(|col| {
            let source_col = source_index(col, draw_width, source_width, fit_to_terminal);
            characters.get(source_col).copied().map(printable).unwrap_or(' ')
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
    let status = status_text(controller, frame_index, total_frames, display);
    let status_line = truncate_to_width(&status, term_width as usize);
    let y = term_height.saturating_sub(1);
    let clear_line = " ".repeat(term_width as usize);
    queue!(stdout, MoveTo(0, y), SetForegroundColor(Color::DarkGrey), SetBackgroundColor(Color::Reset), Print(clear_line), MoveTo(0, y), Print(status_line), ResetColor).context("drawing status line")?;
    Ok(())
}

fn status_text(controller: &AnimationController, frame_index: usize, total_frames: usize, display: &DisplayState) -> String {
    if let Some(value) = &display.seek_input {
        return format!("Go to frame (1-{total_frames}): {value}_  Enter / Esc");
    }
    let playback_state = format!("{:?}", controller.state()).to_lowercase();
    let loop_mode = match controller.loop_mode() {
        LoopMode::Loop => "loop",
        LoopMode::Once => "once",
    };
    let (range_start, range_end) = controller.range_frames();
    let mut fields = Vec::new();
    if display.bar.frame {
        fields.push(format!("frame {}/{}", frame_index + 1, total_frames));
    }
    if let Some((columns, rows, mode)) = display.grid.filter(|_| display.bar.fit) {
        fields.push(format!("{columns}x{rows} {}", mode.label()));
    }
    if display.bar.range {
        fields.push(format!("range {}-{}", range_start + 1, range_end + 1));
    }
    if display.bar.state {
        fields.push(playback_state);
    }
    if display.bar.fps {
        fields.push(format!("{} fps", controller.fps()));
    }
    if display.bar.loop_mode {
        fields.push(loop_mode.to_string());
    }
    if display.bar.fit {
        fields.push(format!("fit:{}", if display.fit_to_terminal {"on"} else {"off"}));
    }
    if display.bar.color {
        fields.push(format!("color:{}", match display.mode { ColorMode::Full => "on", ColorMode::Project => "project", ColorMode::Mono => "mono" }));
    }
    if display.bar.audio {
        fields.push(format!("audio:{}", display.audio_status));
    }
    if display.bar.controls {
        fields.push("[space] play/pause [←/→] step [home/end] range [+/-] fps [f] fit [a/p] auto/map [ / ] width [g] go [c] color [m] mute [l] loop [q] quit".to_string());
    }
    fields.join(" | ")
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

fn is_playback_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".bin", ".cframes", ".csmv", ".cfpk", ".bin.gz", ".json.gz", ".json"].iter().any(|ext| lower.ends_with(ext))
}

fn find_standalone_binary(directory: &Path) -> Result<Option<PathBuf>> {
    if !collect_frame_paths(directory, "txt", true)?.is_empty() || !collect_frame_paths(directory, "cframe", false)?.is_empty() {
        return Ok(None);
    }

    let mut binaries = Vec::new();
    for entry in fs::read_dir(directory)
        .with_context(|| format!("reading frame directory {}", directory.display()))?
    {
        let entry = entry.with_context(|| format!("reading entry in {}", directory.display()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()).is_some_and(is_playback_filename) {
            binaries.push(path);
        }
    }
    // A transport manifest names its tile files. A sibling JSON metadata file
    // is common in older exports and must not displace an existing binary.
    if binaries.len() > 1 {
        let mut primary = Vec::new();
        for path in &binaries {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or_default().to_ascii_lowercase();
            let input = Input::Path(path.clone());
            if name.ends_with(".json") || name.ends_with(".json.gz") {
                if matches!(Frames::open(&input), Ok(Frames::Tiles(_))) { primary.push(path.clone()); }
            } else if name.ends_with(".bin.gz") {
                let file = input::Bytes::open(&input)?;
                if tiles::is_archive(&file.read(0..file.len.min(20))?) { primary.push(path.clone()); }
            } else {
                primary.push(path.clone());
            }
        }
        if !primary.is_empty() { binaries = primary; }
    }
    binaries.sort();
    match binaries.len() {
        0 => Ok(None),
        1 => Ok(binaries.pop()),
        _ => bail!("Multiple packed binaries found in {}; pass one as the playback path", directory.display()),
    }
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
    fn parses_the_legacy_direct_playback_form() {
        let cli = Cli::try_parse_from(["casciit", "frames", "--once"]).unwrap();

        assert!(cli.command.is_none());
        assert_eq!(cli.playback.path, PathBuf::from("frames"));
        assert!(cli.playback.once);
    }

    #[test]
    fn parses_the_named_save_command() {
        let cli = Cli::try_parse_from(["casciit", "save", "frames", "demo-reel"]).unwrap();

        assert!(matches!(cli.command, Some(Command::Save {source, name: Some(name)}) if source == PathBuf::from("frames") && name == "demo-reel"));
    }

    #[test]
    fn parses_the_save_command_without_a_name() {
        let cli = Cli::try_parse_from(["casciit", "save", "test/eth"]).unwrap();

        assert!(matches!(cli.command, Some(Command::Save {source, name: None}) if source == PathBuf::from("test/eth")));
    }

    #[test]
    fn infers_the_final_directory_name_after_validating_frames() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("test/eth");
        let save_path = temp.path().join("library");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("frame_0001.txt"), "hello").unwrap();

        let (name, destination) = save_animation(&source, None, &save_path).unwrap();

        assert_eq!(name, "eth");
        assert_eq!(destination, save_path.join("eth"));
        assert_eq!(fs::read_to_string(destination.join("frame_0001.txt")).unwrap(), "hello");
    }

    #[test]
    fn infers_a_binary_name_without_its_extension() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("family-guy_cframes.bin");
        fs::write(&source, full_cframe_pack(&[encoded_cframe(b'A', [1, 2, 3])])).unwrap();

        let (name, _) = save_animation(&source, None, &temp.path().join("library")).unwrap();

        assert_eq!(name, "family-guy_cframes");
    }

    #[test]
    fn refuses_to_save_a_directory_without_readable_frames() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("not-frames");
        let save_path = temp.path().join("library");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("README.md"), "not an animation").unwrap();

        assert!(save_animation(&source, None, &save_path).is_err());
        assert!(!save_path.exists());
    }

    #[test]
    fn validates_the_bundled_frame_fixture_before_saving() {
        validate_animation_source(Path::new("test/familyguy")).unwrap();
    }

    #[test]
    fn parses_the_save_path_setting_command() {
        let cli = Cli::try_parse_from(["casciit", "config", "set", "save-path", "library"]).unwrap();

        assert!(matches!(cli.command, Some(Command::Config(ConfigArgs {command: Some(ConfigCommand::Set {setting: ConfigSetting::SavePath {path}})})) if path == PathBuf::from("library")));
    }

    #[test]
    fn parses_the_default_color_setting_command() {
        let enabled = Cli::try_parse_from(["casciit", "config", "set", "color", "on"]).unwrap();
        let disabled = Cli::try_parse_from(["casciit", "config", "set", "color", "off"]).unwrap();

        assert!(matches!(enabled.command, Some(Command::Config(ConfigArgs {command: Some(ConfigCommand::Set {setting: ConfigSetting::Color {value: OnOff::On}})}))));
        assert!(matches!(disabled.command, Some(Command::Config(ConfigArgs {command: Some(ConfigCommand::Set {setting: ConfigSetting::Color {value: OnOff::Off}})}))));
    }

    #[test]
    fn parses_the_default_fps_setting_command() {
        let cli = Cli::try_parse_from(["casciit", "config", "set", "fps", "30"]).unwrap();

        assert!(matches!(cli.command, Some(Command::Config(ConfigArgs {command: Some(ConfigCommand::Set {setting: ConfigSetting::Fps {fps: 30}})}))));
        assert!(Cli::try_parse_from(["casciit", "config", "set", "fps", "0"]).is_err());
        assert!(Cli::try_parse_from(["casciit", "frames", "--fps", "0"]).is_err());
    }

    #[test]
    fn parses_the_default_fit_setting_command() {
        let enabled = Cli::try_parse_from(["casciit", "config", "set", "fit", "on"]).unwrap();
        let disabled = Cli::try_parse_from(["casciit", "config", "set", "fit", "off"]).unwrap();

        assert!(matches!(enabled.command, Some(Command::Config(ConfigArgs {command: Some(ConfigCommand::Set {setting: ConfigSetting::Fit {value: OnOff::On}})}))));
        assert!(matches!(disabled.command, Some(Command::Config(ConfigArgs {command: Some(ConfigCommand::Set {setting: ConfigSetting::Fit {value: OnOff::Off}})}))));
    }

    #[test]
    fn parses_loop_and_information_bar_setting_commands() {
        let looping = Cli::try_parse_from(["casciit", "config", "set", "loop", "off"]).unwrap();
        let hidden = Cli::try_parse_from(["casciit", "config", "set", "hide-bar", "on"]).unwrap();
        let frame = Cli::try_parse_from(["casciit", "config", "set", "bar", "frame", "off"]).unwrap();

        assert!(matches!(looping.command, Some(Command::Config(ConfigArgs {command: Some(ConfigCommand::Set {setting: ConfigSetting::Loop {value: OnOff::Off}})}))));
        assert!(matches!(hidden.command, Some(Command::Config(ConfigArgs {command: Some(ConfigCommand::Set {setting: ConfigSetting::HideBar {value: OnOff::On}})}))));
        assert!(matches!(frame.command, Some(Command::Config(ConfigArgs {command: Some(ConfigCommand::Set {setting: ConfigSetting::Bar {field: BarField::Frame, value: OnOff::Off}})}))));
    }

    #[test]
    fn playback_uses_the_color_setting_unless_explicitly_overridden() {
        let default_args = Cli::try_parse_from(["casciit", "frames"]).unwrap().playback;
        let color_args = Cli::try_parse_from(["casciit", "frames", "--color"]).unwrap().playback;
        let monochrome_args = Cli::try_parse_from(["casciit", "frames", "--no-color"]).unwrap().playback;
        let mut app_settings = settings::Settings {
            name: settings::APP_NAME.to_string(),
            version: settings::APP_VERSION.to_string(),
            binary_path: PathBuf::from("casciit"),
            save_path: PathBuf::from("animations"),
            color: true,
            fps: settings::DEFAULT_FPS,
            fit: true,
            loop_playback: true,
            hide_bar: false,
            bar: settings::BarSettings::default(),
            schema_version: settings::SETTINGS_SCHEMA_VERSION,
        };

        assert!(playback_uses_color(&default_args, &app_settings));
        assert!(!playback_uses_color(&monochrome_args, &app_settings));
        app_settings.color = false;
        assert!(!playback_uses_color(&default_args, &app_settings));
        assert!(playback_uses_color(&color_args, &app_settings));
    }

    #[test]
    fn playback_fps_prefers_cli_then_project_then_settings() {
        let app_settings = settings::Settings {
            name: settings::APP_NAME.to_string(),
            version: settings::APP_VERSION.to_string(),
            binary_path: PathBuf::from("casciit"),
            save_path: PathBuf::from("animations"),
            color: false,
            fps: 30,
            fit: true,
            loop_playback: true,
            hide_bar: false,
            bar: settings::BarSettings::default(),
            schema_version: settings::SETTINGS_SCHEMA_VERSION,
        };

        assert_eq!(playback_fps(Some(60), Some(24), &app_settings), 60);
        assert_eq!(playback_fps(None, Some(24), &app_settings), 24);
        assert_eq!(playback_fps(None, None, &app_settings), 30);
    }

    #[test]
    fn playback_uses_the_fit_setting_unless_explicitly_overridden() {
        let default_args = Cli::try_parse_from(["casciit", "frames"]).unwrap().playback;
        let fit_args = Cli::try_parse_from(["casciit", "frames", "--fit"]).unwrap().playback;
        let native_args = Cli::try_parse_from(["casciit", "frames", "--no-fit"]).unwrap().playback;
        let mut app_settings = settings::Settings {
            name: settings::APP_NAME.to_string(),
            version: settings::APP_VERSION.to_string(),
            binary_path: PathBuf::from("casciit"),
            save_path: PathBuf::from("animations"),
            color: false,
            fps: settings::DEFAULT_FPS,
            fit: true,
            loop_playback: true,
            hide_bar: false,
            bar: settings::BarSettings::default(),
            schema_version: settings::SETTINGS_SCHEMA_VERSION,
        };

        assert!(playback_fits_terminal(&default_args, &app_settings));
        assert!(!playback_fits_terminal(&native_args, &app_settings));
        app_settings.fit = false;
        assert!(!playback_fits_terminal(&default_args, &app_settings));
        assert!(playback_fits_terminal(&fit_args, &app_settings));
        assert!(Cli::try_parse_from(["casciit", "frames", "--fit", "--no-fit"]).is_err());
    }

    #[test]
    fn playback_uses_loop_and_bar_settings_unless_overridden() {
        let default_args = Cli::try_parse_from(["casciit", "frames"]).unwrap().playback;
        let once_args = Cli::try_parse_from(["casciit", "frames", "--once"]).unwrap().playback;
        let loop_args = Cli::try_parse_from(["casciit", "frames", "--loop"]).unwrap().playback;
        let hidden_args = Cli::try_parse_from(["casciit", "frames", "--hide-bar"]).unwrap().playback;
        let shown_args = Cli::try_parse_from(["casciit", "frames", "--show-bar"]).unwrap().playback;
        let mut app_settings = settings::Settings {
            name: settings::APP_NAME.to_string(),
            version: settings::APP_VERSION.to_string(),
            binary_path: PathBuf::from("casciit"),
            save_path: PathBuf::from("animations"),
            color: true,
            fps: settings::DEFAULT_FPS,
            fit: true,
            loop_playback: true,
            hide_bar: false,
            bar: settings::BarSettings::default(),
            schema_version: settings::SETTINGS_SCHEMA_VERSION,
        };

        assert!(playback_loops(&default_args, &app_settings));
        assert!(!playback_loops(&once_args, &app_settings));
        assert!(!playback_hides_bar(&default_args, &app_settings));
        assert!(playback_hides_bar(&hidden_args, &app_settings));
        app_settings.loop_playback = false;
        app_settings.hide_bar = true;
        assert!(!playback_loops(&default_args, &app_settings));
        assert!(playback_loops(&loop_args, &app_settings));
        assert!(playback_hides_bar(&default_args, &app_settings));
        assert!(!playback_hides_bar(&shown_args, &app_settings));
        assert!(Cli::try_parse_from(["casciit", "frames", "--once", "--loop"]).is_err());
        assert!(Cli::try_parse_from(["casciit", "frames", "--show-bar", "--hide-bar"]).is_err());
    }

    #[test]
    fn every_information_bar_field_can_be_hidden_independently() {
        let mut bar = settings::BarSettings::default();
        for field in [BarField::Frame, BarField::Range, BarField::State, BarField::Fps, BarField::Loop, BarField::Fit, BarField::Color, BarField::Audio, BarField::Controls] {
            set_bar_field(&mut bar, field, false);
        }

        assert_eq!(bar, settings::BarSettings {frame: false, range: false, state: false, fps: false, loop_mode: false, fit: false, color: false, audio: false, controls: false});
    }

    #[test]
    fn status_text_only_contains_visible_bar_fields() {
        let mut controller = AnimationController::new(30);
        controller.set_frame_count(2);
        controller.play();
        let mut display = DisplayState {
            colors: ProjectDetails::default().frame_colors(),
            has_any_color: true,
            audio_status: "no".into(), mode: ColorMode::Full, grid: None, seek_input: None,
            fit_to_terminal: true,
            hide_bar: false,
            bar: settings::BarSettings::default(),
        };

        let complete = status_text(&controller, 0, 2, &display);
        assert!(complete.contains("frame 1/2"));
        assert!(complete.contains("30 fps"));
        assert!(complete.contains("audio:no"));
        display.bar.frame = false;
        display.bar.fps = false;
        display.bar.audio = false;
        let filtered = status_text(&controller, 0, 2, &display);
        assert!(!filtered.contains("frame 1/2"));
        assert!(!filtered.contains("30 fps"));
        assert!(!filtered.contains("audio:no"));
    }

    #[test]
    fn rejects_conflicting_color_overrides() {
        assert!(Cli::try_parse_from(["casciit", "frames", "--color", "--no-color"]).is_err());
    }

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

    fn load_packed_frames(path: &Path, color: bool) -> Result<Vec<Frame>> {
        let mut frames = Frames::open(&Input::from_path(path)?)?;
        (0..frames.count()).map(|index| frames.frame(index, 0, color)).collect()
    }

    fn frames_from_binary_data(bytes: &[u8], color: bool) -> Result<Vec<Frame>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.bin");
        fs::write(&path, bytes)?;
        load_packed_frames(&path, color)
    }

    fn encoded_cframe(character: u8, background: [u8; 3]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&[character, 10, 20, 30]);
        data.push(cascii_core_view::CFRAME_EXT_FLAG_HAS_BG);
        data.extend_from_slice(&background);
        data
    }

    fn full_cframe_pack(frames: &[Vec<u8>]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"CFPK");
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&(frames.len() as u32).to_le_bytes());
        for frame in frames {
            data.extend_from_slice(&(frame.len() as u32).to_le_bytes());
            data.extend_from_slice(frame);
        }
        data
    }

    #[test]
    fn decodes_cascii_studio_full_cframe_packs() {
        let data = full_cframe_pack(&[
            encoded_cframe(b'A', [40, 50, 60]),
            encoded_cframe(b'B', [70, 80, 90]),
        ]);

        let frames = frames_from_binary_data(&data, true).unwrap();

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].content, "A\n");
        assert_eq!(frames[1].content, "B\n");
        assert_eq!(frames[0].cframe.as_ref().unwrap().bg_rgb_at(0, 0), Some((40, 50, 60)));
        assert_eq!(frames[1].cframe.as_ref().unwrap().bg_rgb_at(0, 0), Some((70, 80, 90)));
    }

    #[test]
    fn rejects_truncated_cascii_studio_full_cframe_packs() {
        let mut data = full_cframe_pack(&[encoded_cframe(b'A', [40, 50, 60])]);
        data.pop();

        assert!(frames_from_binary_data(&data, true).is_err());
    }

    #[test]
    fn saved_binary_files_are_discovered_for_named_playback() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("studio-export.bin");
        let library_path = temp.path().join("library");
        fs::write(&source, full_cframe_pack(&[encoded_cframe(b'A', [1, 2, 3])])).unwrap();
        library::save(&source, "demo", &library_path).unwrap();

        let saved = library::resolve(Path::new("demo"), &library_path);
        let binary = find_standalone_binary(&saved).unwrap().unwrap();
        let frames = load_packed_frames(&binary, true).unwrap();

        assert_eq!(binary.file_name().unwrap(), "studio-export.bin");
        assert_eq!(frames[0].content, "A\n");
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

        let frames = frames_from_binary_data(&data, true).unwrap();

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

        let frames = frames_from_binary_data(&data, false).unwrap();

        assert_eq!(frames[0].content, "X\n");
        assert!(frames[0].cframe.is_none());
    }
}

#[cfg(test)]
mod playback_tests;
