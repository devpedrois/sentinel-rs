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

/// CSV flattening strategy for variable-length fields (disks, network interfaces):
///
/// Each snapshot is flattened into a single CSV row. Fixed columns come first:
///   timestamp, cpu_global, ram_total, ram_used, ram_percent
///
/// Then disk columns are emitted in order, indexed from 0:
///   disk_mount_0, disk_percent_0, disk_mount_1, disk_percent_1, ...
///
/// Then network columns, also indexed from 0:
///   net_interface_0, net_tx_delta_0, net_rx_delta_0, net_interface_1, ...
///
/// The header is generated from the first snapshot in each file. If later snapshots
/// have more disks/interfaces, extra values are silently dropped. If they have fewer,
/// empty values are padded. This is a pragmatic trade-off: on most systems the number
/// of disks and interfaces stays constant during a monitoring session.
///
/// All fields are serialized via the `csv` crate (RFC 4180 compliant), so values
/// containing commas, quotes, or newlines are properly escaped.
pub struct CsvWriter {
    buffer: Vec<SystemSnapshot>,
    buffer_capacity: usize,
    output_dir: PathBuf,
    current_file: PathBuf,
    file_started_at: Option<SystemTime>,
    rotation_policy: RotationPolicy,
    header_written: bool,
    num_disks: usize,
    num_interfaces: usize,
}

impl CsvWriter {
    pub fn new(config: &AppConfig) -> Self {
        let current_file = config.output_dir.join("sentinel.csv");
        let file_started_at = detect_file_started_at(&current_file);
        Self {
            buffer: Vec::with_capacity(config.buffer_size),
            buffer_capacity: config.buffer_size,
            output_dir: config.output_dir.clone(),
            current_file,
            file_started_at,
            rotation_policy: RotationPolicy {
                max_file_size_bytes: config.max_file_size_mb.saturating_mul(1024 * 1024),
                rotate_every: config.rotate_every,
                max_files: config.max_files,
            },
            header_written: false,
            num_disks: 0,
            num_interfaces: 0,
        }
    }

    #[cfg(test)]
    fn with_params(
        output_dir: PathBuf,
        buffer_capacity: usize,
        rotation_policy: RotationPolicy,
    ) -> Self {
        let current_file = output_dir.join("sentinel.csv");
        let file_started_at = detect_file_started_at(&current_file);
        Self {
            buffer: Vec::with_capacity(buffer_capacity),
            buffer_capacity,
            output_dir,
            current_file,
            file_started_at,
            rotation_policy,
            header_written: false,
            num_disks: 0,
            num_interfaces: 0,
        }
    }

    async fn load_existing_layout(&mut self) -> anyhow::Result<()> {
        if self.header_written {
            return Ok(());
        }

        let metadata = match fs::metadata(&self.current_file).await {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        if metadata.len() == 0 {
            return Ok(());
        }

        let content = fs::read_to_string(&self.current_file).await?;
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_reader(content.as_bytes());

        let Some(record) = reader.records().next() else {
            return Ok(());
        };

        let header = record?;
        self.num_disks = header
            .iter()
            .filter(|column| column.starts_with("disk_mount_"))
            .count();
        self.num_interfaces = header
            .iter()
            .filter(|column| column.starts_with("net_interface_"))
            .count();
        self.header_written = true;

        Ok(())
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

    fn required_layout(&self) -> (usize, usize) {
        self.buffer
            .iter()
            .fold((0, 0), |(max_disks, max_ifaces), snapshot| {
                (
                    max_disks.max(snapshot.disks.len()),
                    max_ifaces.max(snapshot.network.len()),
                )
            })
    }

    async fn rotate_for_schema_expansion(
        &mut self,
        required_disks: usize,
        required_interfaces: usize,
    ) -> anyhow::Result<()> {
        if !self.header_written
            || (required_disks <= self.num_disks && required_interfaces <= self.num_interfaces)
        {
            return Ok(());
        }

        let previous_disks = self.num_disks;
        let previous_interfaces = self.num_interfaces;
        let metadata = match fs::metadata(&self.current_file).await {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };

        if metadata.as_ref().is_some_and(|metadata| metadata.len() > 0) {
            let force_rotation_policy = RotationPolicy {
                max_file_size_bytes: 0,
                rotate_every: None,
                max_files: self.rotation_policy.max_files,
            };

            let rotation_result = check_and_rotate(
                &self.current_file,
                &force_rotation_policy,
                "csv",
                self.file_started_at,
            )
            .await?;

            if rotation_result.rotated {
                self.reopen_active_file().await?;
            }
        }

        self.header_written = false;
        self.num_disks = 0;
        self.num_interfaces = 0;
        self.file_started_at = Some(SystemTime::now());

        info!(
            previous_disks,
            previous_interfaces,
            required_disks,
            required_interfaces,
            "CSV schema expanded, starting a new file layout"
        );

        Ok(())
    }

    fn build_header_record(&self) -> Vec<String> {
        let mut cols = vec![
            "timestamp".to_string(),
            "cpu_global".to_string(),
            "ram_total".to_string(),
            "ram_used".to_string(),
            "ram_percent".to_string(),
        ];

        for i in 0..self.num_disks {
            cols.push(format!("disk_mount_{i}"));
            cols.push(format!("disk_percent_{i}"));
        }

        for i in 0..self.num_interfaces {
            cols.push(format!("net_interface_{i}"));
            cols.push(format!("net_tx_delta_{i}"));
            cols.push(format!("net_rx_delta_{i}"));
        }

        cols
    }

    fn flatten_snapshot(&self, snapshot: &SystemSnapshot) -> Vec<String> {
        let mut fields: Vec<String> = vec![
            snapshot.timestamp.clone(),
            format!("{:.2}", snapshot.cpu.global_usage),
            snapshot.memory.total_bytes.to_string(),
            snapshot.memory.used_bytes.to_string(),
            format!("{:.2}", snapshot.memory.usage_percent),
        ];

        for i in 0..self.num_disks {
            if let Some(disk) = snapshot.disks.get(i) {
                fields.push(disk.mount_point.clone());
                fields.push(format!("{:.2}", disk.usage_percent));
            } else {
                for _ in 0..2 {
                    fields.push(String::new());
                }
            }
        }

        for i in 0..self.num_interfaces {
            if let Some(net) = snapshot.network.get(i) {
                fields.push(net.interface.clone());
                fields.push(net.tx_delta.to_string());
                fields.push(net.rx_delta.to_string());
            } else {
                for _ in 0..3 {
                    fields.push(String::new());
                }
            }
        }

        fields
    }

    fn serialize_records(&self, records: &[Vec<String>]) -> anyhow::Result<Vec<u8>> {
        let mut wtr = csv::Writer::from_writer(Vec::new());
        for record in records {
            wtr.write_record(record)?;
        }
        wtr.flush()?;
        Ok(wtr.into_inner()?)
    }

    async fn flush_buffer(&mut self) -> anyhow::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        fs::create_dir_all(&self.output_dir).await?;
        if self.file_started_at.is_none() {
            self.file_started_at = Some(SystemTime::now());
        }
        self.load_existing_layout().await?;
        let (required_disks, required_interfaces) = self.required_layout();
        self.rotate_for_schema_expansion(required_disks, required_interfaces)
            .await?;

        let mut records: Vec<Vec<String>> = Vec::new();

        if !self.header_written {
            self.num_disks = required_disks;
            self.num_interfaces = required_interfaces;
            records.push(self.build_header_record());
            self.header_written = true;
        }

        for snapshot in &self.buffer {
            records.push(self.flatten_snapshot(snapshot));
        }

        let csv_bytes = self.serialize_records(&records)?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.current_file)
            .await?;

        file.write_all(&csv_bytes).await?;
        file.flush().await?;

        info!(
            count = self.buffer.len(),
            path = %self.current_file.display(),
            "Flushed snapshots to CSV"
        );

        let rotation_result = check_and_rotate(
            &self.current_file,
            &self.rotation_policy,
            "csv",
            self.file_started_at,
        )
        .await?;
        if rotation_result.rotated {
            self.header_written = false;
            self.num_disks = 0;
            self.num_interfaces = 0;
            self.reopen_active_file().await?;
            self.file_started_at = Some(SystemTime::now());
        }

        self.buffer.clear();
        Ok(())
    }
}

#[async_trait]
impl LogWriter for CsvWriter {
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
    use crate::models::{CpuMetrics, DiskMetrics, MemoryMetrics, NetworkMetrics};
    use crate::persistence::rotation::RotationPolicy;

    fn make_snapshot_with_peripherals(
        ts: &str,
        disks: Vec<DiskMetrics>,
        network: Vec<NetworkMetrics>,
    ) -> SystemSnapshot {
        SystemSnapshot {
            timestamp: ts.to_string(),
            cpu: CpuMetrics {
                global_usage: 42.0,
                per_core: vec![42.0],
            },
            memory: MemoryMetrics {
                total_bytes: 16_000_000_000,
                used_bytes: 8_000_000_000,
                available_bytes: 8_000_000_000,
                usage_percent: 50.0,
            },
            disks,
            network,
        }
    }

    fn sample_disk(mount: &str) -> DiskMetrics {
        DiskMetrics {
            mount_point: mount.to_string(),
            total_bytes: 500_000_000_000,
            used_bytes: 250_000_000_000,
            available_bytes: 250_000_000_000,
            usage_percent: 50.0,
        }
    }

    fn sample_net(iface: &str) -> NetworkMetrics {
        NetworkMetrics {
            interface: iface.to_string(),
            tx_bytes: 1000,
            rx_bytes: 2000,
            tx_delta: 100,
            rx_delta: 200,
        }
    }

    fn test_policy() -> RotationPolicy {
        RotationPolicy {
            max_file_size_bytes: 10 * 1024 * 1024,
            rotate_every: None,
            max_files: 10,
        }
    }

    #[tokio::test]
    async fn writes_header_on_first_flush() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = CsvWriter::with_params(dir.path().to_path_buf(), 1, test_policy());

        let snap =
            make_snapshot_with_peripherals("t1", vec![sample_disk("C:")], vec![sample_net("eth0")]);
        writer.write(&snap).await.unwrap();

        let content = fs::read_to_string(dir.path().join("sentinel.csv"))
            .await
            .unwrap();
        let lines: Vec<&str> = content.lines().collect();

        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "timestamp,cpu_global,ram_total,ram_used,ram_percent,disk_mount_0,disk_percent_0,net_interface_0,net_tx_delta_0,net_rx_delta_0"
        );
    }

    #[tokio::test]
    async fn does_not_repeat_header_on_subsequent_flushes() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = CsvWriter::with_params(dir.path().to_path_buf(), 1, test_policy());

        let snap = make_snapshot_with_peripherals("t1", vec![], vec![]);
        writer.write(&snap).await.unwrap();

        let snap2 = make_snapshot_with_peripherals("t2", vec![], vec![]);
        writer.write(&snap2).await.unwrap();

        let content = fs::read_to_string(dir.path().join("sentinel.csv"))
            .await
            .unwrap();
        let header_count = content
            .lines()
            .filter(|l| l.starts_with("timestamp,"))
            .count();

        assert_eq!(header_count, 1);
    }

    #[tokio::test]
    async fn flattens_multiple_disks_and_interfaces() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = CsvWriter::with_params(dir.path().to_path_buf(), 1, test_policy());

        let snap = make_snapshot_with_peripherals(
            "t1",
            vec![sample_disk("C:"), sample_disk("D:")],
            vec![sample_net("eth0"), sample_net("wlan0")],
        );
        writer.write(&snap).await.unwrap();

        let content = fs::read_to_string(dir.path().join("sentinel.csv"))
            .await
            .unwrap();
        let header = content.lines().next().unwrap();

        assert_eq!(
            header,
            "timestamp,cpu_global,ram_total,ram_used,ram_percent,disk_mount_0,disk_percent_0,disk_mount_1,disk_percent_1,net_interface_0,net_tx_delta_0,net_rx_delta_0,net_interface_1,net_tx_delta_1,net_rx_delta_1"
        );

        let data = content
            .lines()
            .nth(1)
            .unwrap()
            .split(',')
            .collect::<Vec<_>>();
        assert_eq!(
            data,
            vec![
                "t1",
                "42.00",
                "16000000000",
                "8000000000",
                "50.00",
                "C:",
                "50.00",
                "D:",
                "50.00",
                "eth0",
                "100",
                "200",
                "wlan0",
                "100",
                "200",
            ]
        );
    }

    #[tokio::test]
    async fn buffer_does_not_flush_before_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = CsvWriter::with_params(dir.path().to_path_buf(), 3, test_policy());

        let snap = make_snapshot_with_peripherals("t1", vec![], vec![]);
        writer.write(&snap).await.unwrap();
        writer.write(&snap).await.unwrap();

        let file_path = dir.path().join("sentinel.csv");
        assert!(!fs::try_exists(&file_path).await.unwrap());
    }

    #[tokio::test]
    async fn escapes_fields_with_commas() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = CsvWriter::with_params(dir.path().to_path_buf(), 1, test_policy());

        let snap =
            make_snapshot_with_peripherals("t1", vec![sample_disk("/mnt/data,backup")], vec![]);
        writer.write(&snap).await.unwrap();

        let content = fs::read_to_string(dir.path().join("sentinel.csv"))
            .await
            .unwrap();

        let mut rdr = csv::Reader::from_reader(content.as_bytes());
        let record = rdr.records().next().unwrap().unwrap();
        assert_eq!(&record[5], "/mnt/data,backup");
    }

    #[tokio::test]
    async fn rotates_after_write_when_flush_pushes_file_over_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = CsvWriter::with_params(
            dir.path().to_path_buf(),
            1,
            RotationPolicy {
                max_file_size_bytes: 1,
                rotate_every: None,
                max_files: 10,
            },
        );

        let snap = make_snapshot_with_peripherals("t1", vec![], vec![]);
        writer.write(&snap).await.unwrap();

        let current_path = dir.path().join("sentinel.csv");
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
                .map(|name| name.starts_with("sentinel_") && name.ends_with(".csv"))
                .unwrap_or(false)
            {
                rotated_files.push(path);
            }
        }

        assert_eq!(rotated_files.len(), 1);
    }

    #[tokio::test]
    async fn does_not_duplicate_header_when_reusing_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let current_path = dir.path().join("sentinel.csv");
        fs::write(
            &current_path,
            "timestamp,cpu_global,ram_total,ram_used,ram_percent\nold,1,2,3,4\n",
        )
        .await
        .unwrap();

        let mut writer = CsvWriter::with_params(dir.path().to_path_buf(), 1, test_policy());
        let snap = make_snapshot_with_peripherals("t1", vec![], vec![]);
        writer.write(&snap).await.unwrap();

        let content = fs::read_to_string(&current_path).await.unwrap();
        let header_count = content
            .lines()
            .filter(|line| line.starts_with("timestamp,cpu_global,"))
            .count();

        assert_eq!(header_count, 1);
    }

    #[tokio::test]
    async fn uses_largest_topology_seen_in_the_buffer_for_the_first_header() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = CsvWriter::with_params(dir.path().to_path_buf(), 2, test_policy());

        let first = make_snapshot_with_peripherals("t1", vec![sample_disk("C:")], vec![]);
        let second = make_snapshot_with_peripherals(
            "t2",
            vec![sample_disk("C:"), sample_disk("D:")],
            vec![sample_net("eth0")],
        );

        writer.write(&first).await.unwrap();
        writer.write(&second).await.unwrap();

        let content = fs::read_to_string(dir.path().join("sentinel.csv"))
            .await
            .unwrap();
        let header = content.lines().next().unwrap();
        let rows: Vec<Vec<String>> = content
            .lines()
            .skip(1)
            .map(|line| {
                line.split(',')
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .collect();

        assert!(
            header.contains("disk_mount_1"),
            "header should expand to include the second disk column: {header}"
        );
        assert!(
            header.contains("net_interface_0"),
            "header should expand to include the first network interface column: {header}"
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][7], "");
        assert_eq!(rows[1][7], "D:");
        assert_eq!(rows[1][9], "eth0");
    }

    #[tokio::test]
    async fn rotates_current_file_before_appending_rows_that_need_a_larger_schema() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = CsvWriter::with_params(dir.path().to_path_buf(), 1, test_policy());

        let first = make_snapshot_with_peripherals("t1", vec![sample_disk("C:")], vec![]);
        let second = make_snapshot_with_peripherals(
            "t2",
            vec![sample_disk("C:"), sample_disk("D:")],
            vec![sample_net("eth0")],
        );

        writer.write(&first).await.unwrap();
        writer.write(&second).await.unwrap();

        let current_path = dir.path().join("sentinel.csv");
        let current_content = fs::read_to_string(&current_path).await.unwrap();
        let current_lines: Vec<&str> = current_content.lines().collect();

        assert_eq!(current_lines.len(), 2);
        assert!(
            current_lines[0].contains("disk_mount_1"),
            "new active file should have the expanded schema: {}",
            current_lines[0]
        );
        assert!(
            current_lines[0].contains("net_interface_0"),
            "new active file should have the expanded schema: {}",
            current_lines[0]
        );
        assert!(
            current_lines[1].contains("D:"),
            "new active file should keep the extra disk data: {}",
            current_lines[1]
        );
        assert!(
            current_lines[1].contains("eth0"),
            "new active file should keep the extra interface data: {}",
            current_lines[1]
        );

        let mut entries = fs::read_dir(dir.path()).await.unwrap();
        let mut rotated_files = Vec::new();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("sentinel_") && name.ends_with(".csv"))
                .unwrap_or(false)
            {
                rotated_files.push(path);
            }
        }

        assert_eq!(rotated_files.len(), 1);

        let rotated_content = fs::read_to_string(&rotated_files[0]).await.unwrap();
        let rotated_lines: Vec<&str> = rotated_content.lines().collect();
        assert_eq!(rotated_lines.len(), 2);
        assert!(
            !rotated_lines[0].contains("disk_mount_1"),
            "old rotated file should preserve the original schema: {}",
            rotated_lines[0]
        );
        assert!(
            rotated_lines[1].contains("t1"),
            "rotated file should keep the original row: {}",
            rotated_lines[1]
        );
    }
}
