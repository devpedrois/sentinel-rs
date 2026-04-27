use sysinfo::{System, MINIMUM_CPU_UPDATE_INTERVAL};
use thiserror::Error;

use crate::models::CpuMetrics;

use super::MetricCollector;

#[derive(Debug, Error)]
pub enum CpuCollectorError {
    #[error("no CPU data available")]
    NoCpuData,
}

pub struct CpuCollector {
    system: System,
    initialized: bool,
}

impl Default for CpuCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuCollector {
    pub fn new() -> Self {
        let mut system = System::new();
        system.refresh_cpu_usage();
        Self {
            system,
            initialized: false,
        }
    }

    pub fn collect_sync(&mut self) -> Result<CpuMetrics, CpuCollectorError> {
        if !self.initialized {
            std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
        }
        self.system.refresh_cpu_usage();
        self.initialized = true;

        let cpus = self.system.cpus();
        if cpus.is_empty() {
            return Err(CpuCollectorError::NoCpuData);
        }

        let per_core: Vec<f32> = cpus.iter().map(|cpu| cpu.cpu_usage()).collect();
        let global_usage = self.system.global_cpu_usage();

        Ok(CpuMetrics {
            global_usage,
            per_core,
        })
    }
}

impl MetricCollector for CpuCollector {
    type Output = CpuMetrics;
    type Error = CpuCollectorError;

    async fn collect(&mut self) -> Result<Self::Output, Self::Error> {
        self.collect_sync()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_cpu_metrics_within_valid_range() {
        let mut collector = CpuCollector::new();

        let metrics = collector
            .collect_sync()
            .expect("should collect CPU metrics");

        assert!(
            (0.0..=100.0).contains(&metrics.global_usage),
            "global CPU usage {} should be between 0 and 100",
            metrics.global_usage
        );
        assert!(
            !metrics.per_core.is_empty(),
            "should have at least one core"
        );
        for (i, usage) in metrics.per_core.iter().enumerate() {
            assert!(
                (0.0..=100.0).contains(usage),
                "core {} usage {} should be between 0 and 100",
                i,
                usage
            );
        }
    }
}
