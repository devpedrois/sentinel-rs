pub mod cpu;
pub mod disk;
pub mod memory;
pub mod network;

use std::sync::{Arc, Mutex};

use chrono::Utc;
use tracing::error;

use crate::models::{CpuMetrics, MemoryMetrics, SystemSnapshot};

pub use self::cpu::CpuCollector;
pub use self::disk::DiskCollector;
pub use self::memory::MemoryCollector;
pub use self::network::NetworkCollector;

#[allow(async_fn_in_trait)]
pub trait MetricCollector {
    type Output;
    type Error;

    async fn collect(&mut self) -> Result<Self::Output, Self::Error>;
}

pub struct SystemCollector {
    cpu: Arc<Mutex<CpuCollector>>,
    memory: Arc<Mutex<MemoryCollector>>,
    disk: Arc<Mutex<DiskCollector>>,
    network: Arc<Mutex<NetworkCollector>>,
}

impl Default for SystemCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemCollector {
    pub fn new() -> Self {
        Self {
            cpu: Arc::new(Mutex::new(CpuCollector::new())),
            memory: Arc::new(Mutex::new(MemoryCollector::new())),
            disk: Arc::new(Mutex::new(DiskCollector::new())),
            network: Arc::new(Mutex::new(NetworkCollector::new())),
        }
    }

    pub async fn collect(&mut self) -> SystemSnapshot {
        let cpu = Arc::clone(&self.cpu);
        let memory = Arc::clone(&self.memory);
        let disk = Arc::clone(&self.disk);
        let network = Arc::clone(&self.network);

        let snapshot_result = tokio::task::spawn_blocking(move || {
            let mut cpu = match cpu.lock() {
                Ok(collector) => collector,
                Err(err) => {
                    error!(error = %err, "CPU collector lock poisoned");
                    return SystemSnapshot {
                        timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                        cpu: CpuMetrics {
                            global_usage: 0.0,
                            per_core: Vec::new(),
                        },
                        memory: MemoryMetrics {
                            total_bytes: 0,
                            used_bytes: 0,
                            available_bytes: 0,
                            usage_percent: 0.0,
                        },
                        disks: Vec::new(),
                        network: Vec::new(),
                    };
                }
            };
            let mut memory = match memory.lock() {
                Ok(collector) => collector,
                Err(err) => {
                    error!(error = %err, "memory collector lock poisoned");
                    return SystemSnapshot {
                        timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                        cpu: CpuMetrics {
                            global_usage: 0.0,
                            per_core: Vec::new(),
                        },
                        memory: MemoryMetrics {
                            total_bytes: 0,
                            used_bytes: 0,
                            available_bytes: 0,
                            usage_percent: 0.0,
                        },
                        disks: Vec::new(),
                        network: Vec::new(),
                    };
                }
            };
            let mut disk = match disk.lock() {
                Ok(collector) => collector,
                Err(err) => {
                    error!(error = %err, "disk collector lock poisoned");
                    return SystemSnapshot {
                        timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                        cpu: CpuMetrics {
                            global_usage: 0.0,
                            per_core: Vec::new(),
                        },
                        memory: MemoryMetrics {
                            total_bytes: 0,
                            used_bytes: 0,
                            available_bytes: 0,
                            usage_percent: 0.0,
                        },
                        disks: Vec::new(),
                        network: Vec::new(),
                    };
                }
            };
            let mut network = match network.lock() {
                Ok(collector) => collector,
                Err(err) => {
                    error!(error = %err, "network collector lock poisoned");
                    return SystemSnapshot {
                        timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                        cpu: CpuMetrics {
                            global_usage: 0.0,
                            per_core: Vec::new(),
                        },
                        memory: MemoryMetrics {
                            total_bytes: 0,
                            used_bytes: 0,
                            available_bytes: 0,
                            usage_percent: 0.0,
                        },
                        disks: Vec::new(),
                        network: Vec::new(),
                    };
                }
            };

            SystemSnapshot {
                timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                cpu: cpu.collect_sync().unwrap_or_else(|err| {
                    error!(error = %err, "CPU collection failed");
                    CpuMetrics {
                        global_usage: 0.0,
                        per_core: Vec::new(),
                    }
                }),
                memory: memory.collect_sync().unwrap_or_else(|err| {
                    error!(error = %err, "memory collection failed");
                    MemoryMetrics {
                        total_bytes: 0,
                        used_bytes: 0,
                        available_bytes: 0,
                        usage_percent: 0.0,
                    }
                }),
                disks: disk.collect_sync().unwrap_or_else(|err| {
                    error!(error = %err, "disk collection failed");
                    Vec::new()
                }),
                network: network.collect_sync().unwrap_or_else(|err| {
                    error!(error = %err, "network collection failed");
                    Vec::new()
                }),
            }
        })
        .await;

        match snapshot_result {
            Ok(snapshot) => snapshot,
            Err(err) => {
                error!(error = %err, "blocking collection task panicked");
                SystemSnapshot {
                    timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                    cpu: CpuMetrics {
                        global_usage: 0.0,
                        per_core: Vec::new(),
                    },
                    memory: MemoryMetrics {
                        total_bytes: 0,
                        used_bytes: 0,
                        available_bytes: 0,
                        usage_percent: 0.0,
                    },
                    disks: Vec::new(),
                    network: Vec::new(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
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

        let snapshot = collector.collect().await;

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
}
