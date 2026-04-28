pub mod logger;
pub mod webhook;

use serde::Serialize;
use thiserror::Error;

use crate::config::AppConfig;
use crate::models::SystemSnapshot;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum AlertType {
    Cpu,
    Ram,
    Collector,
}

#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub alert_type: AlertType,
    pub current_value: f32,
    pub threshold: f32,
    pub consecutive_readings: u32,
    pub timestamp: String,
    pub message: Option<String>,
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("webhook request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("webhook returned unexpected status: {0}")]
    UnexpectedStatus(reqwest::StatusCode),
}

#[allow(async_fn_in_trait)]
pub trait AlertDispatcher {
    async fn dispatch(&self, alert: &Alert) -> Result<(), DispatchError>;
}

#[derive(Debug)]
struct ResourceState {
    consecutive_count: u32,
    alerted: bool,
}

impl ResourceState {
    fn new() -> Self {
        Self {
            consecutive_count: 0,
            alerted: false,
        }
    }

    fn update(&mut self, above_threshold: bool, required: u32) -> bool {
        if above_threshold {
            self.consecutive_count += 1;
            if self.consecutive_count >= required && !self.alerted {
                self.alerted = true;
                return true;
            }
        } else {
            self.consecutive_count = 0;
            self.alerted = false;
        }
        false
    }
}

#[derive(Debug)]
pub struct AlertEvaluator {
    cpu_state: ResourceState,
    ram_state: ResourceState,
    collector_state: ResourceState,
}

impl Default for AlertEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl AlertEvaluator {
    pub fn new() -> Self {
        Self {
            cpu_state: ResourceState::new(),
            ram_state: ResourceState::new(),
            collector_state: ResourceState::new(),
        }
    }

    pub fn evaluate(&mut self, snapshot: &SystemSnapshot, config: &AppConfig) -> Vec<Alert> {
        let mut alerts = Vec::new();

        self.collector_state.update(false, 1);

        let cpu_above = snapshot.cpu.global_usage >= f32::from(config.cpu_threshold);
        if self.cpu_state.update(cpu_above, config.alert_consecutive) {
            alerts.push(Alert {
                alert_type: AlertType::Cpu,
                current_value: snapshot.cpu.global_usage,
                threshold: f32::from(config.cpu_threshold),
                consecutive_readings: self.cpu_state.consecutive_count,
                timestamp: snapshot.timestamp.clone(),
                message: None,
            });
        }

        let ram_above = snapshot.memory.usage_percent >= f32::from(config.ram_threshold);
        if self.ram_state.update(ram_above, config.alert_consecutive) {
            alerts.push(Alert {
                alert_type: AlertType::Ram,
                current_value: snapshot.memory.usage_percent,
                threshold: f32::from(config.ram_threshold),
                consecutive_readings: self.ram_state.consecutive_count,
                timestamp: snapshot.timestamp.clone(),
                message: None,
            });
        }

        alerts
    }

    pub fn record_collection_failure(&mut self, timestamp: &str) -> Option<Alert> {
        if self.collector_state.update(true, 1) {
            return Some(Alert {
                alert_type: AlertType::Collector,
                current_value: self.collector_state.consecutive_count as f32,
                threshold: 1.0,
                consecutive_readings: self.collector_state.consecutive_count,
                timestamp: timestamp.to_string(),
                message: Some("system metric collection failed".to_string()),
            });
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CpuMetrics, MemoryMetrics};

    fn make_config(cpu_threshold: u8, ram_threshold: u8, consecutive: u32) -> AppConfig {
        AppConfig {
            interval: std::time::Duration::from_secs(5),
            format: crate::cli::OutputFormat::Json,
            output_dir: std::path::PathBuf::from("./logs"),
            cpu_threshold,
            ram_threshold,
            alert_consecutive: consecutive,
            max_file_size_mb: 10,
            rotate_every: None,
            max_files: 10,
            webhook_url: None,
            buffer_size: 10,
            verbose: false,
        }
    }

    fn make_snapshot(cpu_usage: f32, ram_usage: f32) -> SystemSnapshot {
        SystemSnapshot {
            timestamp: "2026-04-27T12:00:00Z".to_string(),
            cpu: CpuMetrics {
                global_usage: cpu_usage,
                per_core: vec![cpu_usage],
            },
            memory: MemoryMetrics {
                total_bytes: 16_000_000_000,
                used_bytes: 0,
                available_bytes: 0,
                usage_percent: ram_usage,
            },
            disks: Vec::new(),
            network: Vec::new(),
        }
    }

    #[test]
    fn no_alert_when_below_threshold() {
        let mut evaluator = AlertEvaluator::new();
        let config = make_config(90, 85, 3);
        let snapshot = make_snapshot(50.0, 40.0);

        let alerts = evaluator.evaluate(&snapshot, &config);

        assert!(alerts.is_empty());
    }

    #[test]
    fn no_alert_when_above_for_fewer_than_n_readings() {
        let mut evaluator = AlertEvaluator::new();
        let config = make_config(90, 85, 3);
        let snapshot = make_snapshot(95.0, 90.0);

        assert!(evaluator.evaluate(&snapshot, &config).is_empty());
        assert!(evaluator.evaluate(&snapshot, &config).is_empty());
    }

    #[test]
    fn alert_fires_at_exactly_n_consecutive_readings() {
        let mut evaluator = AlertEvaluator::new();
        let config = make_config(90, 85, 3);
        let snapshot = make_snapshot(95.0, 90.0);

        evaluator.evaluate(&snapshot, &config);
        evaluator.evaluate(&snapshot, &config);
        let alerts = evaluator.evaluate(&snapshot, &config);

        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].alert_type, AlertType::Cpu);
        assert_eq!(alerts[0].consecutive_readings, 3);
        assert_eq!(alerts[1].alert_type, AlertType::Ram);
    }

    #[test]
    fn debounce_prevents_repeated_alerts() {
        let mut evaluator = AlertEvaluator::new();
        let config = make_config(90, 85, 3);
        let high = make_snapshot(95.0, 90.0);

        for _ in 0..3 {
            evaluator.evaluate(&high, &config);
        }

        let alerts = evaluator.evaluate(&high, &config);
        assert!(alerts.is_empty());

        let alerts = evaluator.evaluate(&high, &config);
        assert!(alerts.is_empty());
    }

    #[test]
    fn alert_fires_again_after_reset_and_new_consecutive_readings() {
        let mut evaluator = AlertEvaluator::new();
        let config = make_config(90, 85, 3);
        let high = make_snapshot(95.0, 90.0);
        let low = make_snapshot(50.0, 40.0);

        for _ in 0..3 {
            evaluator.evaluate(&high, &config);
        }

        evaluator.evaluate(&low, &config);

        evaluator.evaluate(&high, &config);
        evaluator.evaluate(&high, &config);
        let alerts = evaluator.evaluate(&high, &config);

        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].alert_type, AlertType::Cpu);
        assert_eq!(alerts[1].alert_type, AlertType::Ram);
    }

    #[test]
    fn partial_sequence_resets_on_low_reading() {
        let mut evaluator = AlertEvaluator::new();
        let config = make_config(90, 85, 3);
        let high = make_snapshot(95.0, 90.0);
        let low = make_snapshot(50.0, 40.0);

        evaluator.evaluate(&high, &config);
        evaluator.evaluate(&high, &config);
        evaluator.evaluate(&low, &config);

        evaluator.evaluate(&high, &config);
        evaluator.evaluate(&high, &config);
        let alerts = evaluator.evaluate(&high, &config);

        assert_eq!(alerts.len(), 2);
    }

    #[test]
    fn cpu_alert_independent_of_ram() {
        let mut evaluator = AlertEvaluator::new();
        let config = make_config(90, 85, 3);
        let cpu_high_ram_low = make_snapshot(95.0, 40.0);

        for _ in 0..2 {
            evaluator.evaluate(&cpu_high_ram_low, &config);
        }
        let alerts = evaluator.evaluate(&cpu_high_ram_low, &config);

        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].alert_type, AlertType::Cpu);
    }

    #[test]
    fn fires_immediately_when_consecutive_is_one() {
        let mut evaluator = AlertEvaluator::new();
        let config = make_config(90, 85, 1);
        let high = make_snapshot(95.0, 90.0);

        let alerts = evaluator.evaluate(&high, &config);

        assert_eq!(alerts.len(), 2);
    }

    #[test]
    fn fires_when_value_exactly_equals_threshold() {
        let mut evaluator = AlertEvaluator::new();
        let config = make_config(90, 85, 3);
        let exact = make_snapshot(90.0, 85.0);

        for _ in 0..2 {
            evaluator.evaluate(&exact, &config);
        }
        let alerts = evaluator.evaluate(&exact, &config);

        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].current_value, 90.0);
        assert_eq!(alerts[1].current_value, 85.0);
    }

    #[test]
    fn collection_failure_emits_a_single_health_alert_until_success() {
        let mut evaluator = AlertEvaluator::new();

        let first = evaluator
            .record_collection_failure("2026-04-28T12:00:00Z")
            .expect("first failure should alert");
        let second = evaluator.record_collection_failure("2026-04-28T12:00:01Z");

        assert_eq!(first.alert_type, AlertType::Collector);
        assert_eq!(first.consecutive_readings, 1);
        assert_eq!(
            first.message.as_deref(),
            Some("system metric collection failed")
        );
        assert!(second.is_none());
    }

    #[test]
    fn collection_success_resets_health_alert_debounce() {
        let mut evaluator = AlertEvaluator::new();
        let config = make_config(90, 85, 3);

        evaluator.record_collection_failure("2026-04-28T12:00:00Z");
        assert!(evaluator
            .record_collection_failure("2026-04-28T12:00:01Z")
            .is_none());

        let _ = evaluator.evaluate(&make_snapshot(10.0, 10.0), &config);

        let after_reset = evaluator
            .record_collection_failure("2026-04-28T12:00:02Z")
            .expect("failure after success should alert again");

        assert_eq!(after_reset.alert_type, AlertType::Collector);
        assert_eq!(after_reset.consecutive_readings, 1);
    }
}
