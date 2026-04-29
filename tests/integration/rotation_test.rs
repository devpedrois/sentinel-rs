use std::path::PathBuf;

use sentinel::models::{CpuMetrics, DiskMetrics, MemoryMetrics, NetworkMetrics, SystemSnapshot};

fn make_snapshot(ts: &str) -> SystemSnapshot {
    SystemSnapshot {
        timestamp: ts.to_string(),
        cpu: CpuMetrics {
            global_usage: 55.0,
            per_core: vec![55.0, 60.0],
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

fn make_config(
    output_dir: PathBuf,
    max_file_size_mb: u64,
    max_files: usize,
    buffer_size: usize,
    format: sentinel::cli::OutputFormat,
) -> sentinel::config::AppConfig {
    sentinel::config::AppConfig {
        interval: std::time::Duration::from_secs(1),
        format,
        output_dir,
        cpu_threshold: 90,
        ram_threshold: 85,
        alert_consecutive: 3,
        max_file_size_mb,
        rotate_every: None,
        max_files,
        webhook_url: None,
        buffer_size,
        verbose: false,
    }
}

#[tokio::test]
async fn json_rotation_creates_rotated_files_and_respects_max_files() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let max_files = 3;

    let config = make_config(
        dir.path().to_path_buf(),
        1,
        max_files,
        1,
        sentinel::cli::OutputFormat::Json,
    );
    let mut writer = sentinel::persistence::create_writer(&config);

    let big_snapshot = SystemSnapshot {
        timestamp: "2026-04-28T00:00:00Z".to_string(),
        cpu: CpuMetrics {
            global_usage: 55.0,
            per_core: vec![55.0; 100],
        },
        memory: MemoryMetrics {
            total_bytes: 16_000_000_000,
            used_bytes: 8_000_000_000,
            available_bytes: 8_000_000_000,
            usage_percent: 50.0,
        },
        disks: Vec::new(),
        network: Vec::new(),
    };

    for _ in 0..6000 {
        writer
            .write(&big_snapshot)
            .await
            .expect("write should succeed");
    }
    writer.flush().await.expect("flush should succeed");

    let mut entries: Vec<String> = Vec::new();
    let mut read_dir = tokio::fs::read_dir(dir.path()).await.expect("read dir");
    while let Some(entry) = read_dir.next_entry().await.expect("next entry") {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".jsonl") {
            entries.push(name);
        }
    }

    assert!(
        entries.len() > 1,
        "rotation should have created at least one rotated file, found: {:?}",
        entries
    );
    assert!(
        entries.len() <= max_files,
        "total files ({}) should not exceed max_files ({}), found: {:?}",
        entries.len(),
        max_files,
        entries
    );
}

#[tokio::test]
async fn csv_rotation_creates_rotated_files_and_respects_max_files() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let max_files = 3;

    let config = make_config(
        dir.path().to_path_buf(),
        1,
        max_files,
        1,
        sentinel::cli::OutputFormat::Csv,
    );
    let mut writer = sentinel::persistence::create_writer(&config);

    let disks: Vec<DiskMetrics> = (0..10)
        .map(|i| DiskMetrics {
            mount_point: format!("/mnt/disk{i}"),
            total_bytes: 500_000_000_000,
            used_bytes: 250_000_000_000,
            available_bytes: 250_000_000_000,
            usage_percent: 50.0,
        })
        .collect();

    let networks: Vec<NetworkMetrics> = (0..10)
        .map(|i| NetworkMetrics {
            interface: format!("eth{i}"),
            tx_bytes: 1_000_000,
            rx_bytes: 2_000_000,
            tx_delta: 1000,
            rx_delta: 2000,
        })
        .collect();

    let big_snapshot = SystemSnapshot {
        timestamp: "2026-04-28T00:00:00Z".to_string(),
        cpu: CpuMetrics {
            global_usage: 55.0,
            per_core: vec![55.0; 100],
        },
        memory: MemoryMetrics {
            total_bytes: 16_000_000_000,
            used_bytes: 8_000_000_000,
            available_bytes: 8_000_000_000,
            usage_percent: 50.0,
        },
        disks,
        network: networks,
    };

    for _ in 0..6000 {
        writer
            .write(&big_snapshot)
            .await
            .expect("write should succeed");
    }
    writer.flush().await.expect("flush should succeed");

    let mut entries: Vec<String> = Vec::new();
    let mut read_dir = tokio::fs::read_dir(dir.path()).await.expect("read dir");
    while let Some(entry) = read_dir.next_entry().await.expect("next entry") {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".csv") {
            entries.push(name);
        }
    }

    assert!(
        entries.len() > 1,
        "rotation should have created at least one rotated file, found: {:?}",
        entries
    );
    assert!(
        entries.len() <= max_files,
        "total files ({}) should not exceed max_files ({}), found: {:?}",
        entries.len(),
        max_files,
        entries
    );
}

#[tokio::test]
async fn json_writer_produces_valid_jsonl_output() {
    let dir = tempfile::tempdir().expect("should create temp dir");

    let config = make_config(
        dir.path().to_path_buf(),
        10,
        10,
        2,
        sentinel::cli::OutputFormat::Json,
    );
    let mut writer = sentinel::persistence::create_writer(&config);

    writer
        .write(&make_snapshot("2026-04-28T00:00:01Z"))
        .await
        .expect("write 1");
    writer
        .write(&make_snapshot("2026-04-28T00:00:02Z"))
        .await
        .expect("write 2");
    writer.flush().await.expect("flush");

    let content = tokio::fs::read_to_string(dir.path().join("sentinel.jsonl"))
        .await
        .expect("read file");

    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);

    for line in &lines {
        let parsed: serde_json::Value =
            serde_json::from_str(line).expect("each line should be valid JSON");
        assert!(parsed.get("timestamp").is_some());
        assert!(parsed.get("cpu").is_some());
        assert!(parsed.get("memory").is_some());
    }
}
