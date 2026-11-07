//! Generated directories are disposable; each successful preflight starts clean.
use anyhow::{Context, Result};
use std::{fs, io, path::Path};

/// Remove a previous generation, including edits and unrelated files.
/// Removing a symlink root removes the link itself, never its target.
pub fn reset_directory(root: &Path) -> Result<()> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() => {
            fs::remove_dir_all(root).with_context(|| format!("clear {}", root.display()))?;
        }
        Ok(_) => fs::remove_file(root).with_context(|| format!("clear {}", root.display()))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("inspect {}", root.display())),
    }
    fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))
}

pub fn write_file(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_removes_everything_from_the_previous_generation() {
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("output");
        for name in ["pkg/Edited.java", "unrelated.txt"] {
            write_file(&root.join(name), b"old contents").unwrap();
        }
        reset_directory(&root).unwrap();
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        write_file(&root.join("pkg/New.java"), b"new source").unwrap();
        assert_eq!(fs::read(root.join("pkg/New.java")).unwrap(), b"new source");
    }

    #[cfg(unix)]
    #[test]
    fn reset_removes_symlinks_without_modifying_their_targets() {
        use std::os::unix::fs::symlink;
        let work = tempfile::tempdir().unwrap();
        let outside = work.path().join("outside");
        write_file(&outside.join("kept.txt"), b"untouched").unwrap();
        let root = work.path().join("output");
        fs::create_dir(&root).unwrap();
        symlink(&outside, root.join("nested")).unwrap();
        reset_directory(&root).unwrap();
        assert!(!root.join("nested").exists());
        let alias = work.path().join("alias");
        symlink(&outside, &alias).unwrap();
        reset_directory(&alias).unwrap();
        assert!(!fs::symlink_metadata(&alias).unwrap().is_symlink());
        assert_eq!(fs::read_dir(alias).unwrap().count(), 0);
        assert_eq!(fs::read(outside.join("kept.txt")).unwrap(), b"untouched");
    }
}
