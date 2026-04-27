use std::{path::PathBuf, time::Duration};

use reqwest::Url;
use thiserror::Error;

use crate::cli::{CliArgs, OutputFormat};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub interval: Duration,
    pub format: OutputFormat,
    pub output_dir: PathBuf,
    pub cpu_threshold: u8,
    pub ram_threshold: u8,
    pub alert_consecutive: u32,
    pub max_file_size_mb: u64,
    pub rotate_every: Option<Duration>,
    pub max_files: usize,
    pub webhook_url: Option<Url>,
    pub buffer_size: usize,
    pub verbose: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("invalid webhook URL: {0}")]
    InvalidWebhookUrl(String),
}

impl TryFrom<CliArgs> for AppConfig {
    type Error = ConfigError;

    fn try_from(value: CliArgs) -> Result<Self, Self::Error> {
        let webhook_url = value
            .webhook_url
            .map(|url| Url::parse(&url).map_err(|_| ConfigError::InvalidWebhookUrl(url)))
            .transpose()?;

        Ok(Self {
            interval: Duration::from_secs(value.interval),
            format: value.format,
            output_dir: value.output_dir,
            cpu_threshold: value.cpu_threshold,
            ram_threshold: value.ram_threshold,
            alert_consecutive: value.alert_consecutive,
            max_file_size_mb: value.max_file_size,
            rotate_every: if value.rotate_every == 0 {
                None
            } else {
                Some(Duration::from_secs(value.rotate_every * 60 * 60))
            },
            max_files: value.max_files,
            webhook_url,
            buffer_size: value.buffer_size,
            verbose: value.verbose,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use reqwest::Url;

    use crate::{
        cli::{CliArgs, OutputFormat},
        config::AppConfig,
    };

    fn sample_args() -> CliArgs {
        CliArgs {
            interval: 15,
            format: OutputFormat::Csv,
            output_dir: PathBuf::from("./custom-logs"),
            cpu_threshold: 80,
            ram_threshold: 70,
            alert_consecutive: 4,
            max_file_size: 32,
            rotate_every: 6,
            max_files: 12,
            webhook_url: Some("https://example.com/hook".to_string()),
            buffer_size: 24,
            verbose: true,
        }
    }

    #[test]
    fn converts_cli_values_to_semantic_types() {
        let config = AppConfig::try_from(sample_args()).expect("config conversion should succeed");

        assert_eq!(config.interval, Duration::from_secs(15));
        assert_eq!(config.format, OutputFormat::Csv);
        assert_eq!(config.output_dir, PathBuf::from("./custom-logs"));
        assert_eq!(config.cpu_threshold, 80);
        assert_eq!(config.ram_threshold, 70);
        assert_eq!(config.alert_consecutive, 4);
        assert_eq!(config.max_file_size_mb, 32);
        assert_eq!(config.rotate_every, Some(Duration::from_secs(6 * 60 * 60)));
        assert_eq!(config.max_files, 12);
        assert_eq!(
            config.webhook_url,
            Some(Url::parse("https://example.com/hook").expect("URL should be valid"))
        );
        assert_eq!(config.buffer_size, 24);
        assert!(config.verbose);
    }

    #[test]
    fn maps_disabled_time_rotation_to_none() {
        let mut args = sample_args();
        args.rotate_every = 0;

        let config = AppConfig::try_from(args).expect("config conversion should succeed");

        assert_eq!(config.rotate_every, None);
    }
}
