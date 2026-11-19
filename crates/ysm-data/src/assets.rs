//! Index of image files in the client dump folder.
//!
//! The database stores bare file names (`face_file_name`, `icon_file_name`…);
//! the index maps them to full paths by scanning the dump folder recursively.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Supported image extensions, in the order they are tried when the stored
/// name has a different (or no) extension.
pub const IMAGE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "bmp", "webp", "tga", "dds"];

#[derive(Debug, Default, Clone)]
pub struct AssetIndex {
    root: Option<PathBuf>,
    /// Lower-cased file name → path. The first file found for a name wins.
    files: HashMap<String, PathBuf>,
}

impl AssetIndex {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Scans `root` recursively. A missing folder yields an empty index.
    pub fn build(root: &Path) -> Self {
        if !root.is_dir() {
            return Self::empty();
        }

        let mut files = HashMap::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_cached_key(std::fs::DirEntry::file_name);

            for entry in entries {
                // On Windows the directory listing already contains the file type.
                let is_directory = match entry.file_type() {
                    Ok(kind) if !kind.is_symlink() => kind.is_dir(),
                    _ => entry.path().is_dir(),
                };
                let name = entry.file_name();
                if is_directory {
                    pending.push(entry.path());
                } else if has_image_extension(Path::new(&name))
                    && let Some(name) = name.to_str()
                {
                    files.entry(name.to_lowercase()).or_insert_with(|| entry.path());
                }
            }
        }

        Self { root: Some(root.to_path_buf()), files }
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Finds the file for a stored name, trying other image extensions when
    /// there is no exact match.
    pub fn resolve(&self, file_name: &str) -> Option<&Path> {
        let trimmed = file_name.trim();
        if trimmed.is_empty() {
            return None;
        }
        // Stored names sometimes carry a relative path with either separator.
        let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed).to_lowercase();
        if let Some(path) = self.files.get(&base) {
            return Some(path);
        }

        let stem = match base.rsplit_once('.') {
            Some((stem, _)) if !stem.is_empty() => stem,
            _ => base.as_str(),
        };
        IMAGE_EXTENSIONS.iter().find_map(|ext| self.files.get(&format!("{stem}.{ext}"))).map(PathBuf::as_path)
    }
}

fn has_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.iter().any(|known| known.eq_ignore_ascii_case(ext)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump(name: &str, files: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ysm-assets-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        }
        root
    }

    #[test]
    fn indexes_images_recursively_and_ignores_other_files() {
        let root = dump("recursive", &["a/icon_fire.png", "a/b/face.JPG", "notes.txt", "a/data.rdb"]);
        let index = AssetIndex::build(&root);

        assert_eq!(index.len(), 2);
        assert_eq!(index.root(), Some(root.as_path()));
        assert!(index.resolve("icon_fire.png").is_some());
        assert!(index.resolve("notes.txt").is_none());
    }

    #[test]
    fn lookup_ignores_case_whitespace_and_directories() {
        let root = dump("case", &["Icons/Skill_Heal.png"]);
        let index = AssetIndex::build(&root);

        assert!(index.resolve("  skill_heal.PNG ").is_some());
        assert!(index.resolve("game/icons\\Skill_Heal.png").is_some());
        assert!(index.resolve("").is_none());
        assert!(index.resolve("   ").is_none());
    }

    #[test]
    fn falls_back_to_other_extensions() {
        let root = dump("fallback", &["face_01.dds", "illust.jpg"]);
        let index = AssetIndex::build(&root);

        assert!(index.resolve("face_01.jpg").unwrap().ends_with("face_01.dds"));
        assert!(index.resolve("face_01").unwrap().ends_with("face_01.dds"));
        assert!(index.resolve("illust.png").unwrap().ends_with("illust.jpg"));
        assert!(index.resolve("unknown.png").is_none());
    }

    #[test]
    fn first_file_found_wins_for_duplicate_names() {
        let root = dump("duplicates", &["a/icon.png", "b/icon.png"]);
        let index = AssetIndex::build(&root);
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn missing_folder_gives_an_empty_index() {
        let index = AssetIndex::build(Path::new("Z:/definitely/not/here"));
        assert!(index.is_empty());
        assert_eq!(index.root(), None);
    }
}
