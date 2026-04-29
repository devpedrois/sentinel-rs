use sysinfo::System;
use thiserror::Error;

use crate::models::MemoryMetrics;

use super::MetricCollector;

#[derive(Debug, Error)]
pub enum MemoryCollectorError {
    #[error("total memory reported as zero")]
    ZeroTotalMemory,
    #[error("blocking task panicked: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
}

pub struct MemoryCollector {
    system: System,
}

impl Default for MemoryCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryCollector {
    pub fn new() -> Self {
        Self {
            system: System::new(),
        }
    }

    pub fn collect_sync(&mut self) -> Result<MemoryMetrics, MemoryCollectorError> {
        self.system.refresh_memory();

        let total_bytes = self.system.total_memory();
        if total_bytes == 0 {
            return Err(MemoryCollectorError::ZeroTotalMemory);
        }

        let used_bytes = self.system.used_memory();
        let available_bytes = self.system.available_memory();
        let usage_percent = (used_bytes as f64 / total_bytes as f64 * 100.0) as f32;

        Ok(MemoryMetrics {
            total_bytes,
            used_bytes,
            available_bytes,
            usage_percent,
        })
    }
}

impl MetricCollector for MemoryCollector {
    type Output = MemoryMetrics;
    type Error = MemoryCollectorError;

    async fn collect(&mut self) -> Result<Self::Output, Self::Error> {
        let mut this = std::mem::take(self);
        let (result, returned) = tokio::task::spawn_blocking(move || {
            let r = this.collect_sync();
            (r, this)
        })
        .await?;
        *self = returned;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_memory_metrics_with_valid_values() {
        let mut collector = MemoryCollector::new();

        let metrics = collector
            .collect_sync()
            .expect("should collect memory metrics");

        assert!(metrics.total_bytes > 0, "total memory should be positive");
        assert!(
            metrics.used_bytes <= metrics.total_bytes,
            "used ({}) should not exceed total ({})",
            metrics.used_bytes,
            metrics.total_bytes
        );
        assert!(
            (0.0..=100.0).contains(&metrics.usage_percent),
            "usage percent {} should be between 0 and 100",
            metrics.usage_percent
        );
    }
}
