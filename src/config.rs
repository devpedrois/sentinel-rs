use std::{fmt, path::PathBuf, time::Duration};

use reqwest::Url;
use thiserror::Error;

use crate::cli::{CliArgs, OutputFormat};

#[derive(Clone, PartialEq, Eq)]
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
    #[error("invalid webhook URL")]
    InvalidWebhookUrl,
    #[error("rotate-every is too large to convert from hours to seconds: {0}")]
    RotateEveryTooLarge(u64),
}

impl fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppConfig")
            .field("interval", &self.interval)
            .field("format", &self.format)
            .field("output_dir", &self.output_dir)
            .field("cpu_threshold", &self.cpu_threshold)
            .field("ram_threshold", &self.ram_threshold)
            .field("alert_consecutive", &self.alert_consecutive)
            .field("max_file_size_mb", &self.max_file_size_mb)
            .field("rotate_every", &self.rotate_every)
            .field("max_files", &self.max_files)
            .field(
                "webhook_url",
                &self.webhook_url.as_ref().map(|_| "<redacted>"),
            )
            .field("buffer_size", &self.buffer_size)
            .field("verbose", &self.verbose)
            .finish()
    }
}

impl TryFrom<CliArgs> for AppConfig {
    type Error = ConfigError;

    fn try_from(value: CliArgs) -> Result<Self, Self::Error> {
        let webhook_url = value
            .webhook_url
            .map(|url| Url::parse(&url).map_err(|_| ConfigError::InvalidWebhookUrl))
            .transpose()?;
        let rotate_every = if value.rotate_every == 0 {
            None
        } else {
            let seconds = value
                .rotate_every
                .checked_mul(60 * 60)
                .ok_or(ConfigError::RotateEveryTooLarge(value.rotate_every))?;
            Some(Duration::from_secs(seconds))
        };

        Ok(Self {
            interval: Duration::from_secs(value.interval),
            format: value.format,
            output_dir: value.output_dir,
            cpu_threshold: value.cpu_threshold,
            ram_threshold: value.ram_threshold,
            alert_consecutive: value.alert_consecutive,
            max_file_size_mb: value.max_file_size,
            rotate_every,
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
        config::{AppConfig, ConfigError},
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

    #[test]
    fn rejects_rotate_every_values_that_overflow_seconds() {
        let mut args = sample_args();
        args.rotate_every = u64::MAX / 60;

        let result = AppConfig::try_from(args);

        assert!(matches!(result, Err(ConfigError::RotateEveryTooLarge(_))));
    }

    #[test]
    fn debug_output_redacts_webhook_url() {
        let config = AppConfig::try_from(sample_args()).expect("config conversion should succeed");

        let debug_output = format!("{config:?}");

        assert!(
            !debug_output.contains("https://example.com/hook"),
            "debug output must not expose the full webhook URL"
        );
        assert!(
            debug_output.contains("<redacted>"),
            "debug output should indicate the webhook URL was redacted"
        );
    }

    #[test]
    fn invalid_webhook_url_errors_do_not_echo_the_secret() {
        let mut args = sample_args();
        args.webhook_url = Some("hooks.slack.com/services/T000/B000/very-secret-token".to_string());

        let error_text = AppConfig::try_from(args)
            .expect_err("invalid webhook URL should fail")
            .to_string();

        assert!(
            !error_text.contains("very-secret-token"),
            "invalid webhook URL errors must not echo secret values"
        );
    }
}
