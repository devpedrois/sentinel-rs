use tokio::time::{self, MissedTickBehavior};
use tracing::{error, info, warn};

use crate::alert::logger::LogAlertDispatcher;
use crate::alert::webhook::WebhookDispatcher;
use crate::alert::{AlertDispatcher, AlertEvaluator};
use crate::collector::SystemCollector;
use crate::config::AppConfig;
use crate::persistence;

pub async fn run(config: AppConfig) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(&config.output_dir).await?;

    let mut collector = SystemCollector::new();
    let mut alert_evaluator = AlertEvaluator::new();
    let mut writer = persistence::create_writer(&config);

    let log_dispatcher = LogAlertDispatcher::new();
    let webhook_dispatcher = config.webhook_url.clone().map(WebhookDispatcher::new);

    let mut interval = time::interval(config.interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    info!("Engine started, collecting every {:?}", config.interval);

    loop {
        tokio::select! {
            _ = shutdown_signal() => {
                info!("Shutdown signal received, flushing buffers");
                if let Err(e) = writer.flush().await {
                    error!(error = %e, "Failed to flush writer on shutdown");
                }
                info!("Graceful shutdown complete");
                return Ok(());
            }
            _ = interval.tick() => {
                let snapshot = match collector.collect().await {
                    Ok(snapshot) => snapshot,
                    Err(err) => {
                        error!(error = %err, "System collection failed");
                        if let Some(alert) = alert_evaluator
                            .record_collection_failure(&chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string())
                        {
                            dispatch_alert(&log_dispatcher, webhook_dispatcher.as_ref(), &alert).await;
                        }
                        continue;
                    }
                };

                info!(
                    cpu = format_args!("{:.1}%", snapshot.cpu.global_usage),
                    ram = format_args!("{:.1}%", snapshot.memory.usage_percent),
                    disks = snapshot.disks.len(),
                    interfaces = snapshot.network.len(),
                    "Snapshot collected"
                );

                let alerts = alert_evaluator.evaluate(&snapshot, &config);

                for alert in &alerts {
                    dispatch_alert(&log_dispatcher, webhook_dispatcher.as_ref(), alert).await;
                }

                if let Err(e) = writer.write(&snapshot).await {
                    warn!(error = %e, "Failed to write snapshot to log");
                }
            }
        }
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn dispatch_alert(
    log_dispatcher: &LogAlertDispatcher,
    webhook_dispatcher: Option<&WebhookDispatcher>,
    alert: &crate::alert::Alert,
) {
    if let Err(err) = log_dispatcher.dispatch(alert).await {
        error!(error = %err, alert = ?alert.alert_type, "Log alert dispatch failed");
    }

    if let Some(webhook) = webhook_dispatcher {
        if let Err(err) = webhook.dispatch(alert).await {
            error!(error = %err, alert = ?alert.alert_type, "Webhook alert dispatch failed");
        }
    }
}
