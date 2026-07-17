use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "casciit";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub name: String,
    pub version: String,
    pub binary_path: PathBuf,
    pub save_path: PathBuf,
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
    let mut settings = if paths.settings_file.exists() {
        let contents = fs::read_to_string(&paths.settings_file).with_context(|| format!("reading settings from {}", paths.settings_file.display()))?;
        serde_json::from_str::<Settings>(&contents).with_context(|| format!("parsing settings from {}", paths.settings_file.display()))?
    } else {
        Settings {name: APP_NAME.to_string(), version: APP_VERSION.to_string(), binary_path: binary_path.clone(), save_path: paths.default_save_path.clone()}
    };

    let metadata_changed = settings.name != APP_NAME || settings.version != APP_VERSION || settings.binary_path != binary_path;
    settings.name = APP_NAME.to_string();
    settings.version = APP_VERSION.to_string();
    settings.binary_path = binary_path;

    ensure_directory(&settings.save_path, "saved-animation")?;
    if metadata_changed || !paths.settings_file.exists() {
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
        let old = Settings {name: "old-name".to_string(), version: "0.0.1".to_string(), binary_path: PathBuf::from("old-binary"), save_path: custom_save_path.clone()};
        write(&paths, &old).unwrap();

        let updated = load_or_create_with_binary(&paths, temp.path().join("new-binary")).unwrap();

        assert_eq!(updated.name, APP_NAME);
        assert_eq!(updated.version, APP_VERSION);
        assert_eq!(updated.save_path, custom_save_path);
        assert!(updated.save_path.is_dir());
    }

    #[test]
    fn stores_configured_save_path_as_an_absolute_path() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            settings_file: temp.path().join("config/settings.json"),
            default_save_path: temp.path().join("default"),
        };
        let mut settings = Settings {name: APP_NAME.to_string(), version: APP_VERSION.to_string(), binary_path: temp.path().join("casciit"), save_path: paths.default_save_path.clone()};

        set_save_path(&paths, &mut settings, &temp.path().join("library")).unwrap();

        assert!(settings.save_path.is_absolute());
        assert!(settings.save_path.is_dir());
    }
}
