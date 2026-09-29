//! Atomic writes for archive data, generated pages, and media.

use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

pub(crate) fn part_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

/// Replace a file only after its temporary sibling has been fully written.
/// Archive directories must have a single writer at a time.
pub(crate) async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = part_path(path);
    let result = async {
        let mut file = tokio::fs::File::create(&temporary).await?;
        file.write_all(bytes).await?;
        file.flush().await?;
        drop(file);
        tokio::fs::rename(&temporary, path).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    Ok(result?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replaces_complete_files_and_cleans_up_failed_writes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("posts.json");
        write_atomic(&file, b"old").await.unwrap();
        write_atomic(&file, b"new").await.unwrap();
        assert_eq!(tokio::fs::read(&file).await.unwrap(), b"new");
        assert!(!part_path(&file).exists());

        let directory = dir.path().join("directory");
        tokio::fs::create_dir(&directory).await.unwrap();
        assert!(write_atomic(&directory, b"data").await.is_err());
        assert!(directory.is_dir());
        assert!(!part_path(&directory).exists());
    }
}
