use sysinfo::Disks;
use thiserror::Error;

use crate::models::DiskMetrics;

use super::MetricCollector;

#[derive(Debug, Error)]
pub enum DiskCollectorError {
    #[error("failed to query disk information")]
    QueryFailed,
}

pub struct DiskCollector {
    disks: Disks,
}

impl Default for DiskCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl DiskCollector {
    pub fn new() -> Self {
        Self {
            disks: Disks::new_with_refreshed_list(),
        }
    }

    pub fn collect_sync(&mut self) -> Result<Vec<DiskMetrics>, DiskCollectorError> {
        self.disks.refresh();

        let metrics = self
            .disks
            .iter()
            .map(|disk| {
                let total_bytes = disk.total_space();
                let available_bytes = disk.available_space();
                let used_bytes = total_bytes.saturating_sub(available_bytes);
                let usage_percent = if total_bytes > 0 {
                    (used_bytes as f64 / total_bytes as f64 * 100.0) as f32
                } else {
                    0.0
                };

                DiskMetrics {
                    mount_point: disk.mount_point().to_string_lossy().to_string(),
                    total_bytes,
                    used_bytes,
                    available_bytes,
                    usage_percent,
                }
            })
            .collect();

        Ok(metrics)
    }
}

impl MetricCollector for DiskCollector {
    type Output = Vec<DiskMetrics>;
    type Error = DiskCollectorError;

    async fn collect(&mut self) -> Result<Self::Output, Self::Error> {
        self.collect_sync()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_disk_metrics_with_valid_values() {
        let mut collector = DiskCollector::new();

        let metrics = collector
            .collect_sync()
            .expect("should collect disk metrics");

        assert!(!metrics.is_empty(), "should detect at least one disk");
        for disk in &metrics {
            assert!(
                disk.total_bytes > 0,
                "disk {} total should be positive",
                disk.mount_point
            );
            assert!(
                disk.used_bytes <= disk.total_bytes,
                "disk {} used ({}) should not exceed total ({})",
                disk.mount_point,
                disk.used_bytes,
                disk.total_bytes
            );
            assert!(
                (0.0..=100.0).contains(&disk.usage_percent),
                "disk {} usage percent {} should be between 0 and 100",
                disk.mount_point,
                disk.usage_percent
            );
        }
    }
}
