use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Json,
    Csv,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
#[command(name = "sentinel", version, about = "System resource monitoring CLI")]
pub struct CliArgs {
    #[arg(short = 'i', long, default_value_t = 5, value_parser = parse_interval)]
    pub interval: u64,

    #[arg(short = 'f', long, default_value_t = OutputFormat::Json, value_enum)]
    pub format: OutputFormat,

    #[arg(short = 'o', long, default_value = "./logs")]
    pub output_dir: PathBuf,

    #[arg(long, default_value_t = 90, value_parser = parse_percentage)]
    pub cpu_threshold: u8,

    #[arg(long, default_value_t = 85, value_parser = parse_percentage)]
    pub ram_threshold: u8,

    #[arg(long, default_value_t = 3)]
    pub alert_consecutive: u32,

    #[arg(long, default_value_t = 10, value_parser = parse_max_file_size)]
    pub max_file_size: u64,

    #[arg(long, default_value_t = 0)]
    pub rotate_every: u64,

    #[arg(long, default_value_t = 10)]
    pub max_files: usize,

    #[arg(long)]
    pub webhook_url: Option<String>,

    #[arg(long, default_value_t = 10)]
    pub buffer_size: usize,

    #[arg(short = 'v', long)]
    pub verbose: bool,
}

fn parse_interval(value: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| "interval must be a positive integer".to_string())?;

    if parsed < 1 {
        return Err("interval must be at least 1 second".to_string());
    }

    Ok(parsed)
}

fn parse_percentage(value: &str) -> Result<u8, String> {
    let parsed = value
        .parse::<i16>()
        .map_err(|_| "threshold must be an integer between 0 and 100".to_string())?;

    if !(0..=100).contains(&parsed) {
        return Err("threshold must be between 0 and 100".to_string());
    }

    Ok(parsed as u8)
}

fn parse_max_file_size(value: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| "max-file-size must be a positive integer".to_string())?;

    if parsed < 1 {
        return Err("max-file-size must be at least 1 MB".to_string());
    }

    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::{CliArgs, OutputFormat};
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn parses_documented_defaults() {
        let args = CliArgs::try_parse_from(["sentinel"]).expect("default CLI args should parse");

        assert_eq!(args.interval, 5);
        assert_eq!(args.format, OutputFormat::Json);
        assert_eq!(args.output_dir, PathBuf::from("./logs"));
        assert_eq!(args.cpu_threshold, 90);
        assert_eq!(args.ram_threshold, 85);
        assert_eq!(args.alert_consecutive, 3);
        assert_eq!(args.max_file_size, 10);
        assert_eq!(args.rotate_every, 0);
        assert_eq!(args.max_files, 10);
        assert_eq!(args.webhook_url, None);
        assert_eq!(args.buffer_size, 10);
        assert!(!args.verbose);
    }

    #[test]
    fn rejects_interval_below_minimum() {
        let result = CliArgs::try_parse_from(["sentinel", "--interval", "0"]);

        assert!(result.is_err());
    }

    #[test]
    fn rejects_thresholds_outside_percentage_range() {
        let cpu_result = CliArgs::try_parse_from(["sentinel", "--cpu-threshold", "101"]);
        let ram_result = CliArgs::try_parse_from(["sentinel", "--ram-threshold", "-1"]);

        assert!(cpu_result.is_err());
        assert!(ram_result.is_err());
    }

    #[test]
    fn rejects_max_file_size_below_minimum() {
        let result = CliArgs::try_parse_from(["sentinel", "--max-file-size", "0"]);

        assert!(result.is_err());
    }
}
