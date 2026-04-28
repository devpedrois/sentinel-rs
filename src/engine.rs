use chrono::Utc;
use tokio::time::{self, MissedTickBehavior};
use tracing::{error, info};

use crate::alert::logger::LogAlertDispatcher;
use crate::alert::webhook::WebhookDispatcher;
use crate::alert::{AlertDispatcher, AlertEvaluator};
use crate::collector::SystemCollector;
use crate::config::AppConfig;

pub async fn run(config: AppConfig) -> anyhow::Result<()> {
    let mut collector = SystemCollector::new();
    let mut alert_evaluator = AlertEvaluator::new();

    let log_dispatcher = LogAlertDispatcher::new();
    let webhook_dispatcher = config.webhook_url.clone().map(WebhookDispatcher::new);

    let mut interval = time::interval(config.interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    info!("Engine started, collecting every {:?}", config.interval);

    loop {
        interval.tick().await;

        let snapshot = match collector.collect().await {
            Ok(snapshot) => snapshot,
            Err(err) => {
                error!(error = %err, "System collection failed");
                if let Some(alert) = alert_evaluator
                    .record_collection_failure(&Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string())
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
