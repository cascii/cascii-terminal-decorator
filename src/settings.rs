use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "casciit";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_FPS: u32 = 30;
pub const SETTINGS_SCHEMA_VERSION: u32 = 2;

fn default_fps() -> u32 {
    DEFAULT_FPS
}

fn default_fit() -> bool {
    true
}

fn default_color() -> bool {
    true
}

fn default_loop() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BarSettings {
    pub frame: bool,
    pub range: bool,
    pub state: bool,
    pub fps: bool,
    #[serde(rename = "loop")]
    pub loop_mode: bool,
    pub fit: bool,
    pub color: bool,
    pub audio: bool,
    pub controls: bool,
}

impl Default for BarSettings {
    fn default() -> Self {
        Self {frame: true, range: true, state: true, fps: true, loop_mode: true, fit: true, color: true, audio: true, controls: true}
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub name: String,
    pub version: String,
    pub binary_path: PathBuf,
    pub save_path: PathBuf,
    #[serde(default = "default_color")]
    pub color: bool,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default = "default_fit")]
    pub fit: bool,
    #[serde(rename = "loop", default = "default_loop")]
    pub loop_playback: bool,
    #[serde(default)]
    pub hide_bar: bool,
    #[serde(default)]
    pub bar: BarSettings,
    #[serde(default)]
    pub schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub settings_file: PathBuf,
    pub default_save_path: PathBuf,
}

pub fn app_paths() -> Result<AppPaths> {
    let project_dirs = ProjectDirs::from("com", "cascii", APP_NAME).context("determining the operating system's application directories")?;
    Ok(AppPaths {settings_file: project_dirs.config_dir().join("settings.json"), default_save_path: project_dirs.data_local_dir().join("animations")})
}

pub fn load_or_create(paths: &AppPaths) -> Result<Settings> {
    let binary_path = env::current_exe().context("determining the path to the casciit binary")?;
    load_or_create_with_binary(paths, binary_path)
}

fn load_or_create_with_binary(paths: &AppPaths, binary_path: PathBuf) -> Result<Settings> {
    let (mut settings, mut schema_changed) = if paths.settings_file.exists() {
        let contents = fs::read_to_string(&paths.settings_file).with_context(|| format!("reading settings from {}", paths.settings_file.display()))?;
        let document = serde_json::from_str::<serde_json::Value>(&contents).with_context(|| format!("parsing settings from {}", paths.settings_file.display()))?;
        let schema_changed = document.get("color").is_none() || document.get("fps").is_none() || document.get("fit").is_none() || document.get("loop").is_none() || document.get("hide_bar").is_none() || document.get("bar").is_none() || document.get("schema_version").is_none();
        let settings = serde_json::from_value::<Settings>(document).with_context(|| format!("parsing settings from {}", paths.settings_file.display()))?;
        (settings, schema_changed)
    } else {
        (Settings {name: APP_NAME.to_string(), version: APP_VERSION.to_string(), binary_path: binary_path.clone(), save_path: paths.default_save_path.clone(), color: true, fps: DEFAULT_FPS, fit: true, loop_playback: true, hide_bar: false, bar: BarSettings::default(), schema_version: SETTINGS_SCHEMA_VERSION}, true)
    };

    if settings.schema_version < 1 {
        settings.color = true;
    }
    if settings.schema_version < SETTINGS_SCHEMA_VERSION {
        settings.schema_version = SETTINGS_SCHEMA_VERSION;
        schema_changed = true;
    }

    let metadata_changed = settings.name != APP_NAME || settings.version != APP_VERSION || settings.binary_path != binary_path;
    settings.name = APP_NAME.to_string();
    settings.version = APP_VERSION.to_string();
    settings.binary_path = binary_path;
    if settings.fps == 0 {
        bail!("The default FPS setting must be at least 1");
    }

    ensure_directory(&settings.save_path, "saved-animation")?;
    if metadata_changed || schema_changed || !paths.settings_file.exists() {
        write(paths, &settings)?;
    }
    Ok(settings)
}

pub fn set_save_path(paths: &AppPaths, settings: &mut Settings, requested_path: &Path) -> Result<()> {
    let absolute_path = if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        env::current_dir().context("determining the current directory")?.join(requested_path)
    };
    ensure_directory(&absolute_path, "saved-animation")?;
    settings.save_path = absolute_path;
    write(paths, settings)
}

pub fn set_color(paths: &AppPaths, settings: &mut Settings, enabled: bool) -> Result<()> {
    settings.color = enabled;
    write(paths, settings)
}

pub fn set_fps(paths: &AppPaths, settings: &mut Settings, fps: u32) -> Result<()> {
    if fps == 0 {
        bail!("The default FPS setting must be at least 1");
    }
    settings.fps = fps;
    write(paths, settings)
}

pub fn set_fit(paths: &AppPaths, settings: &mut Settings, enabled: bool) -> Result<()> {
    settings.fit = enabled;
    write(paths, settings)
}

pub fn set_loop(paths: &AppPaths, settings: &mut Settings, enabled: bool) -> Result<()> {
    settings.loop_playback = enabled;
    write(paths, settings)
}

pub fn set_hide_bar(paths: &AppPaths, settings: &mut Settings, hidden: bool) -> Result<()> {
    settings.hide_bar = hidden;
    write(paths, settings)
}

pub fn write(paths: &AppPaths, settings: &Settings) -> Result<()> {
    let parent = paths.settings_file.parent().context("settings path has no parent directory")?;
    ensure_directory(parent, "settings")?;
    let mut json = serde_json::to_string_pretty(settings).context("serializing settings")?;
    json.push('\n');
    fs::write(&paths.settings_file, json).with_context(|| format!("writing settings to {}", paths.settings_file.display()))
}

fn ensure_directory(path: &Path, description: &str) -> Result<()> {
    if path.exists() && !path.is_dir() {
        bail!("The {description} path is not a directory: {}", path.display());
    }
    fs::create_dir_all(path).with_context(|| format!("creating {description} directory {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_settings_with_required_metadata_and_default_save_path() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {settings_file: temp.path().join("config/settings.json"), default_save_path: temp.path().join("data/animations")};
        let binary_path = temp.path().join("bin/casciit");

        let settings = load_or_create_with_binary(&paths, binary_path.clone()).unwrap();

        assert_eq!(settings.name, APP_NAME);
        assert_eq!(settings.version, APP_VERSION);
        assert_eq!(settings.binary_path, binary_path);
        assert_eq!(settings.save_path, paths.default_save_path);
        assert!(settings.color);
        assert_eq!(settings.fps, DEFAULT_FPS);
        assert!(settings.fit);
        assert!(settings.loop_playback);
        assert!(!settings.hide_bar);
        assert_eq!(settings.bar, BarSettings::default());
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert!(settings.save_path.is_dir());
        let saved: Settings = serde_json::from_str(&fs::read_to_string(paths.settings_file).unwrap()).unwrap();
        assert_eq!(saved, settings);
    }

    #[test]
    fn preserves_save_path_while_refreshing_install_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            settings_file: temp.path().join("config/settings.json"),
            default_save_path: temp.path().join("default"),
        };
        let custom_save_path = temp.path().join("custom");
        let old = Settings {name: "old-name".to_string(), version: "0.0.1".to_string(), binary_path: PathBuf::from("old-binary"), save_path: custom_save_path.clone(), color: true, fps: 48, fit: false, loop_playback: false, hide_bar: true, bar: BarSettings::default(), schema_version: SETTINGS_SCHEMA_VERSION};
        write(&paths, &old).unwrap();

        let updated = load_or_create_with_binary(&paths, temp.path().join("new-binary")).unwrap();

        assert_eq!(updated.name, APP_NAME);
        assert_eq!(updated.version, APP_VERSION);
        assert_eq!(updated.save_path, custom_save_path);
        assert!(updated.color);
        assert_eq!(updated.fps, 48);
        assert!(!updated.fit);
        assert!(!updated.loop_playback);
        assert!(updated.hide_bar);
        assert!(updated.save_path.is_dir());
    }

    #[test]
    fn stores_configured_save_path_as_an_absolute_path() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            settings_file: temp.path().join("config/settings.json"),
            default_save_path: temp.path().join("default"),
        };
        let mut settings = Settings {name: APP_NAME.to_string(), version: APP_VERSION.to_string(), binary_path: temp.path().join("casciit"), save_path: paths.default_save_path.clone(), color: false, fps: DEFAULT_FPS, fit: true, loop_playback: true, hide_bar: false, bar: BarSettings::default(), schema_version: SETTINGS_SCHEMA_VERSION};

        set_save_path(&paths, &mut settings, &temp.path().join("library")).unwrap();

        assert!(settings.save_path.is_absolute());
        assert!(settings.save_path.is_dir());
    }

    #[test]
    fn migrates_existing_settings_to_the_color_default() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {settings_file: temp.path().join("config/settings.json"), default_save_path: temp.path().join("default")};
        fs::create_dir_all(paths.settings_file.parent().unwrap()).unwrap();
        let old_settings = serde_json::json!({
            "name": APP_NAME,
            "version": APP_VERSION,
            "binary_path": temp.path().join("casciit"),
            "save_path": temp.path().join("library"),
            "color": false,
            "fps": DEFAULT_FPS,
            "fit": true
        });
        fs::write(&paths.settings_file, serde_json::to_string_pretty(&old_settings).unwrap()).unwrap();

        let settings = load_or_create_with_binary(&paths, temp.path().join("casciit")).unwrap();

        assert!(settings.color);
        assert_eq!(settings.fps, DEFAULT_FPS);
        assert!(settings.fit);
        assert!(settings.loop_playback);
        assert!(!settings.hide_bar);
        assert_eq!(settings.bar, BarSettings::default());
        let saved: serde_json::Value = serde_json::from_str(&fs::read_to_string(paths.settings_file).unwrap()).unwrap();
        assert_eq!(saved["color"], true);
        assert_eq!(saved["fps"], DEFAULT_FPS);
        assert_eq!(saved["fit"], true);
        assert_eq!(saved["loop"], true);
        assert_eq!(saved["hide_bar"], false);
        assert_eq!(saved["bar"]["controls"], true);
        assert_eq!(saved["schema_version"], SETTINGS_SCHEMA_VERSION);
    }

    #[test]
    fn migrates_schema_one_bar_defaults_without_overwriting_color() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {settings_file: temp.path().join("config/settings.json"), default_save_path: temp.path().join("default")};
        fs::create_dir_all(paths.settings_file.parent().unwrap()).unwrap();
        let old_settings = serde_json::json!({
            "name": APP_NAME,
            "version": APP_VERSION,
            "binary_path": temp.path().join("casciit"),
            "save_path": temp.path().join("library"),
            "color": false,
            "fps": DEFAULT_FPS,
            "fit": true,
            "schema_version": 1
        });
        fs::write(&paths.settings_file, serde_json::to_string_pretty(&old_settings).unwrap()).unwrap();

        let settings = load_or_create_with_binary(&paths, temp.path().join("casciit")).unwrap();

        assert!(!settings.color);
        assert!(settings.loop_playback);
        assert!(!settings.hide_bar);
        assert_eq!(settings.bar, BarSettings::default());
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
    }

    #[test]
    fn persists_the_color_preference() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {settings_file: temp.path().join("config/settings.json"), default_save_path: temp.path().join("default")};
        let mut settings = Settings {name: APP_NAME.to_string(), version: APP_VERSION.to_string(), binary_path: temp.path().join("casciit"), save_path: paths.default_save_path.clone(), color: true, fps: DEFAULT_FPS, fit: true, loop_playback: true, hide_bar: false, bar: BarSettings::default(), schema_version: SETTINGS_SCHEMA_VERSION};

        set_color(&paths, &mut settings, false).unwrap();

        let saved = load_or_create_with_binary(&paths, temp.path().join("casciit")).unwrap();
        assert!(!saved.color);
    }

    #[test]
    fn persists_the_default_fps() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {settings_file: temp.path().join("config/settings.json"), default_save_path: temp.path().join("default")};
        let mut settings = Settings {name: APP_NAME.to_string(), version: APP_VERSION.to_string(), binary_path: temp.path().join("casciit"), save_path: paths.default_save_path.clone(), color: false, fps: DEFAULT_FPS, fit: true, loop_playback: true, hide_bar: false, bar: BarSettings::default(), schema_version: SETTINGS_SCHEMA_VERSION};

        set_fps(&paths, &mut settings, 60).unwrap();

        let saved: Settings = serde_json::from_str(&fs::read_to_string(&paths.settings_file).unwrap()).unwrap();
        assert_eq!(saved.fps, 60);
        assert!(set_fps(&paths, &mut settings, 0).is_err());
    }

    #[test]
    fn persists_the_fit_preference() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {settings_file: temp.path().join("config/settings.json"), default_save_path: temp.path().join("default")};
        let mut settings = Settings {name: APP_NAME.to_string(), version: APP_VERSION.to_string(), binary_path: temp.path().join("casciit"), save_path: paths.default_save_path.clone(), color: false, fps: DEFAULT_FPS, fit: true, loop_playback: true, hide_bar: false, bar: BarSettings::default(), schema_version: SETTINGS_SCHEMA_VERSION};

        set_fit(&paths, &mut settings, false).unwrap();

        let saved: Settings = serde_json::from_str(&fs::read_to_string(&paths.settings_file).unwrap()).unwrap();
        assert!(!saved.fit);
    }

    #[test]
    fn persists_loop_and_bar_visibility_preferences() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {settings_file: temp.path().join("config/settings.json"), default_save_path: temp.path().join("default")};
        let mut settings = Settings {name: APP_NAME.to_string(), version: APP_VERSION.to_string(), binary_path: temp.path().join("casciit"), save_path: paths.default_save_path.clone(), color: true, fps: DEFAULT_FPS, fit: true, loop_playback: true, hide_bar: false, bar: BarSettings::default(), schema_version: SETTINGS_SCHEMA_VERSION};

        set_loop(&paths, &mut settings, false).unwrap();
        set_hide_bar(&paths, &mut settings, true).unwrap();

        let saved: Settings = serde_json::from_str(&fs::read_to_string(&paths.settings_file).unwrap()).unwrap();
        assert!(!saved.loop_playback);
        assert!(saved.hide_bar);
        assert_eq!(saved.bar, BarSettings::default());
    }
}
