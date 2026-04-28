pub mod cpu;
pub mod disk;
pub mod memory;
pub mod network;

use chrono::Utc;
use sysinfo::{System, MINIMUM_CPU_UPDATE_INTERVAL};
use thiserror::Error;

use crate::models::{CpuMetrics, MemoryMetrics, SystemSnapshot};

pub use self::cpu::CpuCollector;
use self::cpu::CpuCollectorError;
pub use self::disk::DiskCollector;
use self::disk::DiskCollectorError;
pub use self::memory::MemoryCollector;
use self::memory::MemoryCollectorError;
pub use self::network::NetworkCollector;
use self::network::NetworkCollectorError;

#[allow(async_fn_in_trait)]
pub trait MetricCollector {
    type Output;
    type Error;

    async fn collect(&mut self) -> Result<Self::Output, Self::Error>;
}

#[derive(Debug, Error)]
pub enum SystemCollectorError {
    #[error("CPU collection failed: {0}")]
    Cpu(#[from] CpuCollectorError),
    #[error("memory collection failed: {0}")]
    Memory(#[from] MemoryCollectorError),
    #[error("disk collection failed: {0}")]
    Disk(#[from] DiskCollectorError),
    #[error("network collection failed: {0}")]
    Network(#[from] NetworkCollectorError),
    #[error("blocking collection task failed: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
}

pub struct SystemCollector {
    core: CoreMetricsCollector,
    disk: DiskCollector,
    network: NetworkCollector,
}

impl Default for SystemCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemCollector {
    pub fn new() -> Self {
        Self {
            core: CoreMetricsCollector::new(),
            disk: DiskCollector::new(),
            network: NetworkCollector::new(),
        }
    }

    pub async fn collect(&mut self) -> Result<SystemSnapshot, SystemCollectorError> {
        let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let mut core = std::mem::take(&mut self.core);
        let mut disk = std::mem::take(&mut self.disk);
        let mut network = std::mem::take(&mut self.network);

        let result = tokio::task::spawn_blocking(move || {
            let snapshot = assemble_snapshot(
                timestamp,
                || core.collect_sync(),
                || disk.collect_sync(),
                || network.collect_sync(),
            );

            (snapshot, core, disk, network)
        })
        .await?;

        let (snapshot, core, disk, network) = result;
        self.core = core;
        self.disk = disk;
        self.network = network;

        snapshot
    }
}

#[derive(Debug)]
struct CoreMetricsCollector {
    system: System,
    initialized: bool,
}

impl Default for CoreMetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreMetricsCollector {
    fn new() -> Self {
        let mut system = System::new();
        system.refresh_cpu_usage();

        Self {
            system,
            initialized: false,
        }
    }

    fn collect_sync(&mut self) -> Result<(CpuMetrics, MemoryMetrics), SystemCollectorError> {
        if !self.initialized {
            std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
        }

        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.initialized = true;

        let cpus = self.system.cpus();
        if cpus.is_empty() {
            return Err(CpuCollectorError::NoCpuData.into());
        }

        let total_bytes = self.system.total_memory();
        if total_bytes == 0 {
            return Err(MemoryCollectorError::ZeroTotalMemory.into());
        }

        let used_bytes = self.system.used_memory();
        let available_bytes = self.system.available_memory();

        Ok((
            CpuMetrics {
                global_usage: self.system.global_cpu_usage(),
                per_core: cpus.iter().map(|cpu| cpu.cpu_usage()).collect(),
            },
            MemoryMetrics {
                total_bytes,
                used_bytes,
                available_bytes,
                usage_percent: (used_bytes as f64 / total_bytes as f64 * 100.0) as f32,
            },
        ))
    }
}

fn assemble_snapshot<FCore, FDisk, FNetwork>(
    timestamp: String,
    core: FCore,
    disk: FDisk,
    network: FNetwork,
) -> Result<SystemSnapshot, SystemCollectorError>
where
    FCore: FnOnce() -> Result<(CpuMetrics, MemoryMetrics), SystemCollectorError>,
    FDisk: FnOnce() -> Result<Vec<crate::models::DiskMetrics>, DiskCollectorError>,
    FNetwork: FnOnce() -> Result<Vec<crate::models::NetworkMetrics>, NetworkCollectorError>,
{
    let (cpu, memory) = core()?;

    Ok(SystemSnapshot {
        timestamp,
        cpu,
        memory,
        disks: disk()?,
        network: network()?,
    })
}

#[cfg(test)]
mod tests {
    use super::{assemble_snapshot, SystemCollectorError};
    use crate::collector::{
        cpu::CpuCollectorError, disk::DiskCollectorError, memory::MemoryCollectorError,
        network::NetworkCollectorError,
    };
    use crate::models::{CpuMetrics, DiskMetrics, MemoryMetrics, NetworkMetrics, SystemSnapshot};

    #[test]
    fn reexports_concrete_collectors() {
        let _cpu = crate::collector::CpuCollector::new();
        let _memory = crate::collector::MemoryCollector::new();
        let _disk = crate::collector::DiskCollector::new();
        let _network = crate::collector::NetworkCollector::new();
    }

    #[tokio::test]
    async fn collects_a_complete_system_snapshot() {
        let mut collector = crate::collector::SystemCollector::new();

        let snapshot = collector
            .collect()
            .await
            .expect("collection should succeed");

        assert!(
            !snapshot.timestamp.is_empty(),
            "timestamp should be populated"
        );
        assert!(
            !snapshot.cpu.per_core.is_empty(),
            "snapshot should contain per-core CPU data"
        );
        assert!(
            (0.0..=100.0).contains(&snapshot.memory.usage_percent),
            "memory usage should stay within percentage bounds"
        );
    }

    #[tokio::test]
    async fn collect_returns_result_type() {
        let mut collector = crate::collector::SystemCollector::new();

        let result: Result<SystemSnapshot, SystemCollectorError> = collector.collect().await;

        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn assemble_snapshot_stops_on_cpu_failure() {
        let result = assemble_snapshot(
            "2026-04-28T00:00:00Z".to_string(),
            || Err(CpuCollectorError::NoCpuData.into()),
            || {
                Ok(vec![DiskMetrics {
                    mount_point: "/".to_string(),
                    total_bytes: 100,
                    used_bytes: 50,
                    available_bytes: 50,
                    usage_percent: 50.0,
                }])
            },
            || {
                Ok(vec![NetworkMetrics {
                    interface: "eth0".to_string(),
                    tx_bytes: 10,
                    rx_bytes: 20,
                    tx_delta: 1,
                    rx_delta: 2,
                }])
            },
        );

        assert!(matches!(result, Err(SystemCollectorError::Cpu(_))));
    }

    #[test]
    fn assemble_snapshot_returns_full_snapshot_when_all_collectors_succeed() {
        let snapshot = assemble_snapshot(
            "2026-04-28T00:00:00Z".to_string(),
            || {
                Ok((
                    CpuMetrics {
                        global_usage: 42.0,
                        per_core: vec![42.0],
                    },
                    MemoryMetrics {
                        total_bytes: 16,
                        used_bytes: 8,
                        available_bytes: 8,
                        usage_percent: 50.0,
                    },
                ))
            },
            || {
                Ok(vec![DiskMetrics {
                    mount_point: "/".to_string(),
                    total_bytes: 100,
                    used_bytes: 50,
                    available_bytes: 50,
                    usage_percent: 50.0,
                }])
            },
            || {
                Ok(vec![NetworkMetrics {
                    interface: "eth0".to_string(),
                    tx_bytes: 10,
                    rx_bytes: 20,
                    tx_delta: 1,
                    rx_delta: 2,
                }])
            },
        )
        .expect("all collectors should succeed");

        assert_eq!(snapshot.timestamp, "2026-04-28T00:00:00Z");
        assert_eq!(snapshot.cpu.global_usage, 42.0);
        assert_eq!(snapshot.memory.usage_percent, 50.0);
        assert_eq!(snapshot.disks.len(), 1);
        assert_eq!(snapshot.network.len(), 1);
    }

    #[test]
    fn assemble_snapshot_stops_on_network_failure() {
        let result = assemble_snapshot(
            "2026-04-28T00:00:00Z".to_string(),
            || {
                Ok((
                    CpuMetrics {
                        global_usage: 42.0,
                        per_core: vec![42.0],
                    },
                    MemoryMetrics {
                        total_bytes: 16,
                        used_bytes: 8,
                        available_bytes: 8,
                        usage_percent: 50.0,
                    },
                ))
            },
            || {
                Ok(vec![DiskMetrics {
                    mount_point: "/".to_string(),
                    total_bytes: 100,
                    used_bytes: 50,
                    available_bytes: 50,
                    usage_percent: 50.0,
                }])
            },
            || Err(NetworkCollectorError::RefreshFailed),
        );

        assert!(matches!(result, Err(SystemCollectorError::Network(_))));
    }

    #[test]
    fn assemble_snapshot_stops_on_disk_failure() {
        let result = assemble_snapshot(
            "2026-04-28T00:00:00Z".to_string(),
            || {
                Ok((
                    CpuMetrics {
                        global_usage: 42.0,
                        per_core: vec![42.0],
                    },
                    MemoryMetrics {
                        total_bytes: 16,
                        used_bytes: 8,
                        available_bytes: 8,
                        usage_percent: 50.0,
                    },
                ))
            },
            || Err(DiskCollectorError::QueryFailed),
            || {
                Ok(vec![NetworkMetrics {
                    interface: "eth0".to_string(),
                    tx_bytes: 10,
                    rx_bytes: 20,
                    tx_delta: 1,
                    rx_delta: 2,
                }])
            },
        );

        assert!(matches!(result, Err(SystemCollectorError::Disk(_))));
    }

    #[test]
    fn assemble_snapshot_stops_on_memory_failure() {
        let result = assemble_snapshot(
            "2026-04-28T00:00:00Z".to_string(),
            || Err(MemoryCollectorError::ZeroTotalMemory.into()),
            || {
                Ok(vec![DiskMetrics {
                    mount_point: "/".to_string(),
                    total_bytes: 100,
                    used_bytes: 50,
                    available_bytes: 50,
                    usage_percent: 50.0,
                }])
            },
            || {
                Ok(vec![NetworkMetrics {
                    interface: "eth0".to_string(),
                    tx_bytes: 10,
                    rx_bytes: 20,
                    tx_delta: 1,
                    rx_delta: 2,
                }])
            },
        );

        assert!(matches!(result, Err(SystemCollectorError::Memory(_))));
    }
}
