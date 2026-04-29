use std::collections::HashMap;

use sysinfo::Networks;
use thiserror::Error;

use crate::models::NetworkMetrics;

use super::MetricCollector;

#[derive(Debug, Error)]
pub enum NetworkCollectorError {
    #[error("failed to refresh network data")]
    RefreshFailed,
    #[error("blocking task panicked: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
}

pub struct NetworkCollector {
    networks: Networks,
    previous: HashMap<String, (u64, u64)>,
}

impl Default for NetworkCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkCollector {
    pub fn new() -> Self {
        let networks = Networks::new_with_refreshed_list();
        let previous: HashMap<String, (u64, u64)> = networks
            .iter()
            .map(|(name, data)| {
                (
                    name.to_string(),
                    (data.total_transmitted(), data.total_received()),
                )
            })
            .collect();

        Self { networks, previous }
    }

    pub fn collect_sync(&mut self) -> Result<Vec<NetworkMetrics>, NetworkCollectorError> {
        self.networks.refresh();

        let mut next_previous = HashMap::with_capacity(self.networks.len());
        let mut metrics = Vec::with_capacity(self.networks.len());

        for (name, data) in &self.networks {
            let tx_bytes = data.total_transmitted();
            let rx_bytes = data.total_received();

            let (tx_delta, rx_delta) = self
                .previous
                .get(name.as_str())
                .map(|(prev_tx, prev_rx)| {
                    (
                        tx_bytes.saturating_sub(*prev_tx),
                        rx_bytes.saturating_sub(*prev_rx),
                    )
                })
                .unwrap_or((0, 0));

            next_previous.insert(name.to_string(), (tx_bytes, rx_bytes));

            metrics.push(NetworkMetrics {
                interface: name.to_string(),
                tx_bytes,
                rx_bytes,
                tx_delta,
                rx_delta,
            });
        }

        self.previous = next_previous;

        Ok(metrics)
    }
}

impl MetricCollector for NetworkCollector {
    type Output = Vec<NetworkMetrics>;
    type Error = NetworkCollectorError;

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
    fn collects_network_metrics_with_valid_interfaces() {
        let mut collector = NetworkCollector::new();

        let metrics = collector
            .collect_sync()
            .expect("should collect network metrics");

        for iface in &metrics {
            assert!(
                !iface.interface.is_empty(),
                "interface name should not be empty"
            );
        }
    }

    #[test]
    fn computes_delta_between_consecutive_reads() {
        let mut collector = NetworkCollector::new();

        let first = collector
            .collect_sync()
            .expect("first collection should succeed");

        let second = collector
            .collect_sync()
            .expect("second collection should succeed");

        for iface in &second {
            if let Some(prev) = first.iter().find(|f| f.interface == iface.interface) {
                assert!(
                    iface.tx_bytes >= prev.tx_bytes,
                    "TX bytes should be monotonically non-decreasing"
                );
                assert!(
                    iface.rx_bytes >= prev.rx_bytes,
                    "RX bytes should be monotonically non-decreasing"
                );
            }
        }
    }

    #[test]
    fn treats_missing_previous_reading_as_zero_delta() {
        let mut collector = NetworkCollector::new();
        collector.previous.clear();

        let metrics = collector
            .collect_sync()
            .expect("collection without previous state should succeed");

        for iface in &metrics {
            assert_eq!(
                iface.tx_delta, 0,
                "first visible TX delta for {} should be zero",
                iface.interface
            );
            assert_eq!(
                iface.rx_delta, 0,
                "first visible RX delta for {} should be zero",
                iface.interface
            );
        }
    }
}
