use tracing::error;

use super::{Alert, AlertDispatcher, AlertType, DispatchError};

pub struct LogAlertDispatcher;

impl LogAlertDispatcher {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LogAlertDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl AlertDispatcher for LogAlertDispatcher {
    async fn dispatch(&self, alert: &Alert) -> Result<(), DispatchError> {
        let alert_type = match &alert.alert_type {
            AlertType::Cpu => "CPU",
            AlertType::Ram => "RAM",
            AlertType::Collector => "COLLECTOR",
        };

        match &alert.alert_type {
            AlertType::Collector => {
                error!(
                    alert_type = alert_type,
                    failure_streak = alert.consecutive_readings,
                    timestamp = %alert.timestamp,
                    message = %alert.message.as_deref().unwrap_or("collector failure"),
                    "ALERT: collector health degraded",
                );
            }
            AlertType::Cpu | AlertType::Ram => {
                error!(
                    alert_type = alert_type,
                    current_value = format_args!("{:.1}%", alert.current_value),
                    threshold = format_args!("{:.1}%", alert.threshold),
                    consecutive_readings = alert.consecutive_readings,
                    timestamp = %alert.timestamp,
                    "ALERT: {} usage exceeded threshold", alert_type,
                );
            }
        }

        Ok(())
    }
}
