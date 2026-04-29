pub mod csv_writer;
pub mod json_writer;
pub mod rotation;

use crate::config::AppConfig;
use crate::models::SystemSnapshot;
use async_trait::async_trait;

#[async_trait]
pub trait LogWriter: Send {
    async fn write(&mut self, snapshot: &SystemSnapshot) -> anyhow::Result<()>;
    async fn flush(&mut self) -> anyhow::Result<()>;
}

pub fn create_writer(config: &AppConfig) -> Box<dyn LogWriter> {
    match config.format {
        crate::cli::OutputFormat::Json => Box::new(json_writer::JsonWriter::new(config)),
        crate::cli::OutputFormat::Csv => Box::new(csv_writer::CsvWriter::new(config)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::OutputFormat;
    use crate::config::AppConfig;
    use std::path::PathBuf;
    use std::time::Duration;

    fn make_config(format: OutputFormat) -> AppConfig {
        AppConfig {
            interval: Duration::from_secs(1),
            format,
            output_dir: PathBuf::from("./logs"),
            cpu_threshold: 90,
            ram_threshold: 85,
            alert_consecutive: 3,
            max_file_size_mb: 10,
            rotate_every: None,
            max_files: 10,
            webhook_url: None,
            buffer_size: 10,
            verbose: false,
        }
    }

    #[test]
    fn create_writer_returns_trait_object() {
        let writer: Box<dyn LogWriter> = create_writer(&make_config(OutputFormat::Json));
        drop(writer);
    }
}
