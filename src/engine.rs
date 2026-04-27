use tokio::time::{self, MissedTickBehavior};
use tracing::info;

use crate::collector::SystemCollector;
use crate::config::AppConfig;

pub async fn run(config: AppConfig) -> anyhow::Result<()> {
    let mut collector = SystemCollector::new();

    let mut interval = time::interval(config.interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    info!("Engine started, collecting every {:?}", config.interval);

    loop {
        interval.tick().await;

        let snapshot = collector.collect().await;

        info!(
            cpu = format_args!("{:.1}%", snapshot.cpu.global_usage),
            ram = format_args!("{:.1}%", snapshot.memory.usage_percent),
            disks = snapshot.disks.len(),
            interfaces = snapshot.network.len(),
            "Snapshot collected"
        );
    }
}
