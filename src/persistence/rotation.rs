use std::path::{Path, PathBuf};
use std::time::SystemTime;

use thiserror::Error;
use tokio::fs;
use tracing::{info, warn};

#[derive(Debug, Error)]
pub enum RotationError {
    #[error("failed to query file metadata: {0}")]
    Metadata(#[source] std::io::Error),
    #[error("failed to rename log file: {0}")]
    Rename(#[source] std::io::Error),
    #[error("failed to delete old log file: {0}")]
    Delete(#[source] std::io::Error),
    #[error("failed to read output directory: {0}")]
    ReadDir(#[source] std::io::Error),
}

pub struct RotationPolicy {
    pub max_file_size_bytes: u64,
    pub rotate_every: Option<std::time::Duration>,
    pub max_files: usize,
}

pub struct RotationResult {
    pub rotated: bool,
    pub new_path: Option<PathBuf>,
}

pub async fn check_and_rotate(
    current_path: &Path,
    policy: &RotationPolicy,
    extension: &str,
    active_file_started_at: Option<SystemTime>,
) -> Result<RotationResult, RotationError> {
    let needs_rotation = match fs::metadata(current_path).await {
        Ok(meta) => {
            let size_exceeded = meta.len() >= policy.max_file_size_bytes;
            let time_exceeded =
                has_exceeded_rotation_age(&meta, policy.rotate_every, active_file_started_at);
            size_exceeded || time_exceeded
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RotationResult {
                rotated: false,
                new_path: None,
            })
        }
        Err(e) => return Err(RotationError::Metadata(e)),
    };

    if !needs_rotation {
        return Ok(RotationResult {
            rotated: false,
            new_path: None,
        });
    }

    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S%.3f");
    let dir = current_path.parent().unwrap_or(Path::new("."));
    let rotated_name = format!("sentinel_{timestamp}.{extension}");
    let rotated_path = dir.join(&rotated_name);

    fs::rename(current_path, &rotated_path)
        .await
        .map_err(RotationError::Rename)?;

    info!(from = %current_path.display(), to = %rotated_path.display(), "Log file rotated");

    enforce_retention(dir, policy.max_files, extension).await?;

    Ok(RotationResult {
        rotated: true,
        new_path: Some(rotated_path),
    })
}

fn has_exceeded_rotation_age(
    metadata: &std::fs::Metadata,
    rotate_every: Option<std::time::Duration>,
    active_file_started_at: Option<SystemTime>,
) -> bool {
    let Some(limit) = rotate_every else {
        return false;
    };

    let reference_time = active_file_started_at.or_else(|| rotation_reference_time(metadata));
    let Some(reference_time) = reference_time else {
        return false;
    };

    SystemTime::now()
        .duration_since(reference_time)
        .map(|age| age >= limit)
        .unwrap_or(false)
}

pub(crate) fn detect_file_started_at(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .ok()
        .and_then(|metadata| rotation_reference_time(&metadata))
}

fn rotation_reference_time(metadata: &std::fs::Metadata) -> Option<SystemTime> {
    let created = metadata.created().ok();
    let modified = metadata.modified().ok();

    match (created, modified) {
        (Some(created), Some(modified)) => Some(earlier_system_time(created, modified)),
        (Some(created), None) => Some(created),
        (None, Some(modified)) => Some(modified),
        (None, None) => None,
    }
}

fn earlier_system_time(left: SystemTime, right: SystemTime) -> SystemTime {
    if left.duration_since(right).is_ok() {
        right
    } else {
        left
    }
}

async fn enforce_retention(
    dir: &Path,
    max_files: usize,
    extension: &str,
) -> Result<(), RotationError> {
    let mut entries: Vec<PathBuf> = Vec::new();

    let mut read_dir = fs::read_dir(dir).await.map_err(RotationError::ReadDir)?;
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(RotationError::ReadDir)?
    {
        let path = entry.path();
        let matches_ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == extension)
            .unwrap_or(false);
        let matches_prefix = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| stem == "sentinel" || stem.starts_with("sentinel_"))
            .unwrap_or(false);

        if matches_ext && matches_prefix {
            entries.push(path);
        }
    }

    let max_rotated = max_files.saturating_sub(1);

    if entries.len() <= max_rotated {
        return Ok(());
    }

    entries.sort();

    let to_delete = entries.len() - max_rotated;
    for path in entries.into_iter().take(to_delete) {
        info!(path = %path.display(), "Deleting old log file (retention limit)");
        if let Err(e) = fs::remove_file(&path).await {
            warn!(path = %path.display(), error = %e, "Failed to delete old log file");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    async fn setup_temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("failed to create temp dir")
    }

    fn default_policy(max_size: u64, max_files: usize) -> RotationPolicy {
        RotationPolicy {
            max_file_size_bytes: max_size,
            rotate_every: None,
            max_files,
        }
    }

    #[tokio::test]
    async fn no_rotation_when_file_below_size_limit() {
        let dir = setup_temp_dir().await;
        let file_path = dir.path().join("sentinel.jsonl");
        fs::write(&file_path, "small data").await.unwrap();

        let policy = default_policy(1024 * 1024, 10);
        let result = check_and_rotate(&file_path, &policy, "jsonl", None)
            .await
            .unwrap();

        assert!(!result.rotated);
        assert!(result.new_path.is_none());
    }

    #[tokio::test]
    async fn rotates_when_file_exceeds_size_limit() {
        let dir = setup_temp_dir().await;
        let file_path = dir.path().join("sentinel.jsonl");
        let data = "x".repeat(200);
        fs::write(&file_path, &data).await.unwrap();

        let policy = default_policy(100, 10);
        let result = check_and_rotate(&file_path, &policy, "jsonl", None)
            .await
            .unwrap();

        assert!(result.rotated);
        assert!(result.new_path.is_some());
        assert!(!fs::try_exists(&file_path).await.unwrap());

        let rotated = result.new_path.unwrap();
        assert!(fs::try_exists(&rotated).await.unwrap());
        assert!(rotated
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("sentinel_"));
    }

    #[tokio::test]
    async fn enforces_max_files_retention() {
        let dir = setup_temp_dir().await;
        let max_files = 4;

        for i in 0..6 {
            let name = format!("sentinel_2025-01-0{}T00-00-00.jsonl", i + 1);
            let path = dir.path().join(&name);
            fs::write(&path, format!("data {i}")).await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let current = dir.path().join("sentinel.jsonl");
        let big_data = "x".repeat(200);
        fs::write(&current, &big_data).await.unwrap();

        let policy = default_policy(100, max_files);
        check_and_rotate(&current, &policy, "jsonl", None)
            .await
            .unwrap();

        let mut read_dir = fs::read_dir(dir.path()).await.unwrap();
        let mut names = Vec::new();
        while let Some(entry) = read_dir.next_entry().await.unwrap() {
            let path = entry.path();
            if path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e == "jsonl")
                .unwrap_or(false)
            {
                names.push(path.file_name().unwrap().to_string_lossy().to_string());
            }
        }

        let max_rotated = max_files - 1;
        assert!(names.len() <= max_rotated);
        assert!(!names
            .iter()
            .any(|name| name == "sentinel_2025-01-01T00-00-00.jsonl"));
    }

    #[tokio::test]
    async fn rotates_when_file_age_exceeds_limit() {
        let dir = setup_temp_dir().await;
        let file_path = dir.path().join("sentinel.jsonl");
        fs::write(&file_path, "old data").await.unwrap();

        std::fs::OpenOptions::new()
            .write(true)
            .open(&file_path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(2 * 60 * 60))
            .unwrap();

        let policy = RotationPolicy {
            max_file_size_bytes: 1024 * 1024,
            rotate_every: Some(Duration::from_secs(60 * 60)),
            max_files: 10,
        };

        let result = check_and_rotate(&file_path, &policy, "jsonl", None)
            .await
            .unwrap();

        assert!(result.rotated);
        assert!(result.new_path.is_some());
    }

    #[tokio::test]
    async fn retention_ignores_non_sentinel_files_with_same_extension() {
        let dir = setup_temp_dir().await;
        let max_files = 2;

        fs::write(dir.path().join("other_log.jsonl"), "must survive retention")
            .await
            .unwrap();
        fs::write(
            dir.path().join("sentinel_2025-01-01T00-00-00.jsonl"),
            "old sentinel log",
        )
        .await
        .unwrap();

        let current = dir.path().join("sentinel.jsonl");
        fs::write(&current, "x".repeat(200)).await.unwrap();

        let policy = default_policy(100, max_files);
        check_and_rotate(&current, &policy, "jsonl", None)
            .await
            .unwrap();

        assert!(fs::try_exists(&dir.path().join("other_log.jsonl"))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn no_rotation_when_file_does_not_exist() {
        let dir = setup_temp_dir().await;
        let file_path = dir.path().join("nonexistent.jsonl");

        let policy = default_policy(100, 10);
        let result = check_and_rotate(&file_path, &policy, "jsonl", None)
            .await
            .unwrap();

        assert!(!result.rotated);
    }
}
