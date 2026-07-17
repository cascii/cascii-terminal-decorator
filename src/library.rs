use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

pub fn save(source: &Path, name: &str, save_path: &Path) -> Result<PathBuf> {
    validate_name(name)?;
    let source_metadata = fs::symlink_metadata(source).with_context(|| format!("reading source path {}", source.display()))?;
    let source_type = source_metadata.file_type();
    if source_type.is_symlink() {
        bail!("Symbolic links are not supported when saving animations: {}", source.display());
    }
    if !source_type.is_dir() && !source_type.is_file() {
        bail!("Source must be a file or directory: {}", source.display());
    }

    fs::create_dir_all(save_path).with_context(|| format!("creating save directory {}", save_path.display()))?;
    let destination = save_path.join(name);
    if destination.exists() {
        bail!("An animation named '{name}' already exists at {}", destination.display());
    }
    if source_type.is_dir() {
        let source = source.canonicalize().with_context(|| format!("resolving source directory {}", source.display()))?;
        let save_path = save_path.canonicalize().with_context(|| format!("resolving save directory {}", save_path.display()))?;
        if save_path.starts_with(&source) {
            bail!("Cannot save an animation library or one of its parent directories into itself");
        }
    }

    fs::create_dir(&destination).with_context(|| format!("creating animation directory {}", destination.display()))?;
    let result = if source_type.is_dir() {
        copy_directory_contents(source, &destination)
    } else {
        let file_name = source.file_name().context("source file has no file name")?;
        copy_file(source, &destination.join(file_name))
    };

    if let Err(error) = result {
        let _ = fs::remove_dir_all(&destination);
        return Err(error);
    }
    Ok(destination)
}

pub fn resolve(input: &Path, save_path: &Path) -> PathBuf {
    if input.exists() || !is_valid_name_path(input) {
        return input.to_path_buf();
    }

    let saved = save_path.join(input);
    if saved.is_dir() {
        saved
    } else {
        input.to_path_buf()
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || !is_valid_name_path(Path::new(name)) {
        bail!("Animation name must be one file-name component (for example: demo-reel)");
    }
    Ok(())
}

fn is_valid_name_path(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(value)) if value != OsStr::new(".") && value != OsStr::new("..")) && components.next().is_none()
}

fn copy_directory_contents(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source).with_context(|| format!("reading source directory {}", source.display()))? {
        let entry = entry.with_context(|| format!("reading entry in {}", source.display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().with_context(|| format!("reading file type for {}", source_path.display()))?;

        if file_type.is_symlink() {
            bail!("Symbolic links are not supported when saving animations: {}", source_path.display());
        } else if file_type.is_dir() {
            fs::create_dir(&destination_path).with_context(|| format!("creating directory {}", destination_path.display()))?;
            copy_directory_contents(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            copy_file(&source_path, &destination_path)?;
        } else {
            bail!("Unsupported source entry: {}", source_path.display());
        }
    }
    Ok(())
}

fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    fs::copy(source, destination).with_context(|| format!("copying {} to {}", source.display(), destination.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_a_directory_under_the_requested_name() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let library = temp.path().join("library");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("frame_0001.txt"), "hello").unwrap();
        fs::create_dir(source.join("nested")).unwrap();
        fs::write(source.join("nested/details.txt"), "metadata").unwrap();

        let destination = save(&source, "demo-reel", &library).unwrap();

        assert_eq!(destination, library.join("demo-reel"));
        assert_eq!(fs::read_to_string(destination.join("frame_0001.txt")).unwrap(), "hello");
        assert_eq!(fs::read_to_string(destination.join("nested/details.txt")).unwrap(), "metadata");
    }

    #[test]
    fn refuses_names_that_can_escape_the_save_directory() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("frame.txt");
        fs::write(&source, "frame").unwrap();

        assert!(save(&source, "../outside", &temp.path().join("library")).is_err());
        assert!(save(&source, ".", &temp.path().join("library")).is_err());
        assert!(save(&source, "", &temp.path().join("library")).is_err());
    }

    #[test]
    fn refuses_to_overwrite_an_existing_saved_animation() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let library = temp.path().join("library");
        fs::create_dir(&source).unwrap();
        fs::create_dir_all(library.join("demo")).unwrap();

        assert!(save(&source, "demo", &library).is_err());
    }

    #[test]
    fn refuses_to_copy_a_directory_into_itself() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        fs::create_dir(&library).unwrap();

        assert!(save(temp.path(), "demo", &library).is_err());
    }

    #[test]
    fn resolves_a_saved_name_without_changing_existing_paths() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        let saved = library.join("demo");
        let direct = temp.path().join("direct");
        fs::create_dir_all(&saved).unwrap();
        fs::create_dir(&direct).unwrap();

        assert_eq!(resolve(Path::new("demo"), &library), saved);
        assert_eq!(resolve(&direct, &library), direct);
        assert_eq!(resolve(Path::new("missing/path"), &library), PathBuf::from("missing/path"));
    }
}
