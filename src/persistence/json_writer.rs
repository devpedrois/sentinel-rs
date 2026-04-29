use std::path::PathBuf;
use std::time::SystemTime;

use async_trait::async_trait;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::info;

use crate::config::AppConfig;
use crate::models::SystemSnapshot;
use crate::persistence::rotation::{check_and_rotate, detect_file_started_at, RotationPolicy};
use crate::persistence::LogWriter;

pub struct JsonWriter {
    buffer: Vec<SystemSnapshot>,
    buffer_capacity: usize,
    output_dir: PathBuf,
    current_file: PathBuf,
    file_started_at: Option<SystemTime>,
    rotation_policy: RotationPolicy,
}

impl JsonWriter {
    pub fn new(config: &AppConfig) -> Self {
        let current_file = config.output_dir.join("sentinel.jsonl");
        let file_started_at = detect_file_started_at(&current_file);
        Self {
            buffer: Vec::with_capacity(config.buffer_size),
            buffer_capacity: config.buffer_size,
            output_dir: config.output_dir.clone(),
            current_file,
            file_started_at,
            rotation_policy: RotationPolicy {
                max_file_size_bytes: config.max_file_size_mb * 1024 * 1024,
                rotate_every: config.rotate_every,
                max_files: config.max_files,
            },
        }
    }

    #[cfg(test)]
    fn with_params(
        output_dir: PathBuf,
        buffer_capacity: usize,
        rotation_policy: RotationPolicy,
    ) -> Self {
        let current_file = output_dir.join("sentinel.jsonl");
        let file_started_at = detect_file_started_at(&current_file);
        Self {
            buffer: Vec::with_capacity(buffer_capacity),
            buffer_capacity,
            output_dir,
            current_file,
            file_started_at,
            rotation_policy,
        }
    }

    async fn reopen_active_file(&self) -> anyhow::Result<()> {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.current_file)
            .await?;
        Ok(())
    }

    async fn flush_buffer(&mut self) -> anyhow::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        fs::create_dir_all(&self.output_dir).await?;
        if self.file_started_at.is_none() {
            self.file_started_at = Some(SystemTime::now());
        }

        let mut content = String::new();
        for snapshot in &self.buffer {
            let line = serde_json::to_string(snapshot)?;
            content.push_str(&line);
            content.push('\n');
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.current_file)
            .await?;

        file.write_all(content.as_bytes()).await?;
        file.flush().await?;

        info!(
            count = self.buffer.len(),
            path = %self.current_file.display(),
            "Flushed snapshots to JSONL"
        );

        let rotation_result = check_and_rotate(
            &self.current_file,
            &self.rotation_policy,
            "jsonl",
            self.file_started_at,
        )
        .await?;
        if rotation_result.rotated {
            self.reopen_active_file().await?;
            self.file_started_at = Some(SystemTime::now());
        }

        self.buffer.clear();
        Ok(())
    }
}

#[async_trait]
impl LogWriter for JsonWriter {
    async fn write(&mut self, snapshot: &SystemSnapshot) -> anyhow::Result<()> {
        self.buffer.push(snapshot.clone());
        if self.buffer.len() >= self.buffer_capacity {
            self.flush_buffer().await?;
        }
        Ok(())
    }

    async fn flush(&mut self) -> anyhow::Result<()> {
        self.flush_buffer().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CpuMetrics, MemoryMetrics};
    use crate::persistence::rotation::RotationPolicy;
    use std::time::{Duration, SystemTime};

    fn make_snapshot(ts: &str) -> SystemSnapshot {
        SystemSnapshot {
            timestamp: ts.to_string(),
            cpu: CpuMetrics {
                global_usage: 50.0,
                per_core: vec![50.0],
            },
            memory: MemoryMetrics {
                total_bytes: 16_000_000_000,
                used_bytes: 8_000_000_000,
                available_bytes: 8_000_000_000,
                usage_percent: 50.0,
            },
            disks: Vec::new(),
            network: Vec::new(),
        }
    }

    #[tokio::test]
    async fn buffer_does_not_flush_before_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = JsonWriter::with_params(
            dir.path().to_path_buf(),
            3,
            RotationPolicy {
                max_file_size_bytes: 10 * 1024 * 1024,
                rotate_every: None,
                max_files: 10,
            },
        );

        writer.write(&make_snapshot("t1")).await.unwrap();
        writer.write(&make_snapshot("t2")).await.unwrap();

        let file_path = dir.path().join("sentinel.jsonl");
        assert!(
            !fs::try_exists(&file_path).await.unwrap(),
            "file should not exist before buffer capacity reached"
        );
    }

    #[tokio::test]
    async fn buffer_flushes_at_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = JsonWriter::with_params(
            dir.path().to_path_buf(),
            3,
            RotationPolicy {
                max_file_size_bytes: 10 * 1024 * 1024,
                rotate_every: None,
                max_files: 10,
            },
        );

        writer.write(&make_snapshot("t1")).await.unwrap();
        writer.write(&make_snapshot("t2")).await.unwrap();
        writer.write(&make_snapshot("t3")).await.unwrap();

        let file_path = dir.path().join("sentinel.jsonl");
        assert!(fs::try_exists(&file_path).await.unwrap());

        let content = fs::read_to_string(&file_path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
    }

    #[tokio::test]
    async fn manual_flush_writes_partial_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = JsonWriter::with_params(
            dir.path().to_path_buf(),
            10,
            RotationPolicy {
                max_file_size_bytes: 10 * 1024 * 1024,
                rotate_every: None,
                max_files: 10,
            },
        );

        writer.write(&make_snapshot("t1")).await.unwrap();
        writer.flush().await.unwrap();

        let file_path = dir.path().join("sentinel.jsonl");
        let content = fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content.lines().count(), 1);
    }

    #[tokio::test]
    async fn writes_valid_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = JsonWriter::with_params(
            dir.path().to_path_buf(),
            2,
            RotationPolicy {
                max_file_size_bytes: 10 * 1024 * 1024,
                rotate_every: None,
                max_files: 10,
            },
        );

        writer.write(&make_snapshot("t1")).await.unwrap();
        writer.write(&make_snapshot("t2")).await.unwrap();

        let file_path = dir.path().join("sentinel.jsonl");
        let content = fs::read_to_string(&file_path).await.unwrap();

        for line in content.lines() {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(parsed.get("timestamp").is_some());
            assert!(parsed.get("cpu").is_some());
            assert!(parsed.get("memory").is_some());
        }
    }

    #[tokio::test]
    async fn rotates_after_write_when_flush_pushes_file_over_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = JsonWriter::with_params(
            dir.path().to_path_buf(),
            1,
            RotationPolicy {
                max_file_size_bytes: 1,
                rotate_every: None,
                max_files: 10,
            },
        );

        writer.write(&make_snapshot("t1")).await.unwrap();

        let current_path = dir.path().join("sentinel.jsonl");
        assert!(
            fs::try_exists(&current_path).await.unwrap(),
            "a new active file should be created immediately after rotation"
        );
        assert_eq!(fs::metadata(&current_path).await.unwrap().len(), 0);

        let mut entries = fs::read_dir(dir.path()).await.unwrap();
        let mut rotated_files = Vec::new();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("sentinel_") && name.ends_with(".jsonl"))
                .unwrap_or(false)
            {
                rotated_files.push(path);
            }
        }

        assert_eq!(rotated_files.len(), 1);
    }

    #[tokio::test]
    async fn rotates_existing_old_file_on_first_flush() {
        let dir = tempfile::tempdir().unwrap();
        let current_path = dir.path().join("sentinel.jsonl");
        fs::write(&current_path, "{\"existing\":true}\n")
            .await
            .unwrap();

        let old_timestamp = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&current_path)
            .unwrap()
            .set_modified(old_timestamp)
            .unwrap();

        let mut writer = JsonWriter::with_params(
            dir.path().to_path_buf(),
            1,
            RotationPolicy {
                max_file_size_bytes: 10 * 1024 * 1024,
                rotate_every: Some(Duration::from_secs(60 * 60)),
                max_files: 10,
            },
        );

        writer.write(&make_snapshot("t1")).await.unwrap();

        assert!(
            fs::try_exists(&current_path).await.unwrap(),
            "the active path should be recreated after rotating an expired file"
        );
        assert_eq!(fs::metadata(&current_path).await.unwrap().len(), 0);
    }
}
