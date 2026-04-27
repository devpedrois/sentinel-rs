use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SystemSnapshot {
    pub timestamp: String,
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub disks: Vec<DiskMetrics>,
    pub network: Vec<NetworkMetrics>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CpuMetrics {
    pub global_usage: f32,
    pub per_core: Vec<f32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MemoryMetrics {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub usage_percent: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DiskMetrics {
    pub mount_point: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub usage_percent: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NetworkMetrics {
    pub interface: String,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_delta: u64,
    pub rx_delta: u64,
}
