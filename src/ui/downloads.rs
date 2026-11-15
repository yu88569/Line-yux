use std::path::{Path, PathBuf};

pub fn unique_download_dest(dir: &Path, suggested_name: &str) -> PathBuf {
    let safe = sanitize_filename(suggested_name);
    let candidate = dir.join(&safe);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(&safe)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    let extension = Path::new(&safe)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    for sequence in 1..10_000 {
        let alternative = dir.join(format!("{stem}-{sequence}{extension}"));
        if !alternative.exists() {
            return alternative;
        }
    }
    dir.join(format!("{stem}-dup{extension}"))
}

pub fn sanitize_filename(suggested_name: &str) -> String {
    let safe = suggested_name
        .chars()
        .map(|character| {
            if matches!(character, '/' | '\\' | '\0') || character.is_control() {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let safe = safe.trim().trim_matches('.');
    if safe.is_empty() || matches!(safe, "." | "..") {
        "download.bin".into()
    } else {
        safe.to_string()
    }
}

pub(super) fn write_album_zip(
    files: &[(String, String)],
    dir: &Path,
    title: &str,
) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(!files.is_empty(), "Album has no photos");
    std::fs::create_dir_all(dir)?;
    let name = sanitize_filename(title);
    let (destination, output) = (0..10_000)
        .find_map(|sequence| {
            let destination = dir.join(if sequence == 0 {
                format!("{name}.zip")
            } else {
                format!("{name}-{sequence}.zip")
            });
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
            {
                Ok(file) => Some(Ok((destination, file))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .unwrap_or_else(|| Err(std::io::Error::other("No unused album ZIP filename")))?;
    let result = (|| -> anyhow::Result<()> {
        let mut archive = zip::ZipWriter::new(output);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(true);
        for (index, (oid, path)) in files.iter().enumerate() {
            let mut source = std::fs::File::open(path)?;
            let extension = Path::new(path)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("jpg");
            let name = sanitize_filename(&format!("{:06}-{oid}.{extension}", index + 1));
            archive.start_file(&name, options)?;
            std::io::copy(&mut source, &mut archive)?;
        }
        archive.finish()?.sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&destination);
        return Err(error);
    }
    Ok(destination)
}

pub(super) struct SavedAttachments {
    registry_path: PathBuf,
    destinations: std::collections::HashMap<String, PathBuf>,
}

impl SavedAttachments {
    pub fn load(data_dir: &Path) -> Self {
        let registry_path = data_dir.join("saved-attachments.json");
        let destinations = std::fs::read(&registry_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            registry_path,
            destinations,
        }
    }

    pub fn destination(&self, message_id: &str) -> Option<&Path> {
        self.destinations.get(message_id).map(PathBuf::as_path)
    }

    pub fn record(&mut self, message_id: &str, destination: &Path) -> std::io::Result<()> {
        self.destinations
            .insert(message_id.to_string(), destination.to_path_buf());
        let bytes = serde_json::to_vec(&self.destinations)?;
        if let Some(parent) = self.registry_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = self.registry_path.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, &self.registry_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn album_zip_preserves_originals_and_safe_unique_entries_without_overwriting() {
        let dir = std::env::temp_dir().join(format!("line-album-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.png");
        std::fs::write(&source, b"original image bytes").unwrap();
        let destination = dir.join("saved");
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("อัลบั้ม.zip"), b"existing archive").unwrap();
        let files = vec![
            ("../photo".into(), source.to_str().unwrap().into()),
            ("..\\photo".into(), source.to_str().unwrap().into()),
        ];
        let result = write_album_zip(&files, &destination, "อัลบั้ม").unwrap();
        assert_eq!(result, destination.join("อัลบั้ม-1.zip"));
        let mut archive = zip::ZipArchive::new(std::fs::File::open(result).unwrap()).unwrap();
        assert_eq!(archive.len(), 2);
        let mut names = std::collections::HashSet::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            assert!(entry.enclosed_name().is_some());
            assert!(!entry.name().contains(['/', '\\']));
            assert!(entry.name().ends_with(".png"));
            assert!(names.insert(entry.name().to_string()));
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"original image bytes");
        }
        assert_eq!(std::fs::read(source).unwrap(), b"original image bytes");
        assert_eq!(
            std::fs::read(destination.join("อัลบั้ม.zip")).unwrap(),
            b"existing archive"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn album_zip_removes_failed_output_and_rejects_empty_albums() {
        let dir = std::env::temp_dir().join(format!("line-album-zip-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.jpg");
        std::fs::write(&source, b"image").unwrap();
        let destination = dir.join("saved");
        let files = vec![
            ("ready".into(), source.to_str().unwrap().into()),
            (
                "missing".into(),
                dir.join("missing.png").to_str().unwrap().into(),
            ),
        ];
        assert!(write_album_zip(&files, &destination, "../../album").is_err());
        assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 0);
        assert!(write_album_zip(&[], &destination, "empty").is_err());
        assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sanitizes_paths_and_control_characters() {
        assert_eq!(sanitize_filename("../../secret\n.txt"), "_.._secret_.txt");
        assert_eq!(sanitize_filename(".."), "download.bin");
        assert_eq!(sanitize_filename(""), "download.bin");
    }

    #[test]
    fn chooses_a_non_overwriting_destination() {
        let dir =
            std::env::temp_dir().join(format!("line-gtk-download-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("photo.jpg"), b"existing").unwrap();
        assert_eq!(
            unique_download_dest(&dir, "photo.jpg"),
            dir.join("photo-1.jpg")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restores_latest_saved_destination_without_confusing_cached_files() {
        let dir = std::env::temp_dir().join(format!(
            "line-saved-attachments-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut saved = SavedAttachments::load(&dir);
        assert!(saved.destination("cached-only").is_none());
        saved
            .record("file-42", &dir.join("Downloads/รายงาน.xlsx"))
            .unwrap();
        let latest = dir.join("custom folder/renamed-1.xlsx");
        saved.record("file-42", &latest).unwrap();
        saved
            .record("image-42", &dir.join("Pictures/photo.png"))
            .unwrap();
        let restored = SavedAttachments::load(&dir);
        assert_eq!(restored.destination("file-42"), Some(latest.as_path()));
        assert_eq!(
            restored.destination("image-42"),
            Some(dir.join("Pictures/photo.png").as_path())
        );
        assert!(restored.destination("cached-only").is_none());
        std::fs::write(dir.join("saved-attachments.json"), b"broken json").unwrap();
        assert!(
            SavedAttachments::load(&dir)
                .destination("file-42")
                .is_none()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
