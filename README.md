# Sentinel

[![Build](https://img.shields.io/badge/build-passing-brightgreen)]()
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Sentinel is a high-performance, low-overhead system resource monitoring CLI built in Rust. It collects real-time metrics for CPU, RAM, disk, and network usage, writing structured logs in JSON or CSV format with automatic file rotation and retention policies.

When resource usage exceeds configurable thresholds for consecutive readings, Sentinel fires alerts via structured logging and optional webhook notifications (Discord, Slack, or any HTTP endpoint). The consecutive-reading requirement with debounce logic prevents noisy false positives from transient spikes.

Sentinel runs as a single Windows binary with zero runtime dependencies. It leverages Tokio for asynchronous I/O, buffered writes to minimize disk overhead, and graceful shutdown with signal handling to ensure no data loss.

## Features

- Real-time monitoring of CPU (global + per-core), RAM, disk (per mount point), and network (per interface with TX/RX deltas).
- Configurable collection interval (minimum 1 second).
- Structured output in JSONL or CSV format.
- Automatic log file rotation by size and/or time.
- Configurable file retention with automatic cleanup of old logs.
- Buffered async writes to minimize I/O overhead.
- Threshold-based alerting for CPU and RAM with consecutive-reading debounce.
- Alert dispatch via structured logging (always active) and HTTP webhook (optional).
- Webhook retry with exponential backoff for transient failures.
- Graceful shutdown via Ctrl+C/SIGINT with buffer flush on Windows.
- Windows-only support with the pinned `x86_64-pc-windows-msvc` target.

## Installation

Requires Windows with Rust 1.75+ and Cargo.

```powershell
git clone https://github.com/your-user/sentinel.git
cd sentinel
cargo build --release
```

The compiled binary will be at `target\x86_64-pc-windows-msvc\release\sentinel.exe`.

> Current release target: Windows only. The repository pins `x86_64-pc-windows-msvc` in `.cargo/config.toml`, so the default build output does not use the generic `target/release` path.

## Usage

### Basic monitoring with JSON output

```bash
sentinel --interval 5 --format json --output-dir ./logs
```

Collects a system snapshot every 5 seconds, writing JSONL logs to `./logs/`.

### Monitoring with Discord webhook alerts

```bash
sentinel --interval 10 \
  --cpu-threshold 85 \
  --ram-threshold 80 \
  --alert-consecutive 5 \
  --webhook-url "https://discord.com/api/webhooks/your-webhook-id/your-token"
```

Fires an alert when CPU stays above 85% or RAM above 80% for 5 consecutive readings.

### CSV export with custom rotation

```bash
sentinel --format csv \
  --output-dir /var/log/sentinel \
  --max-file-size 50 \
  --rotate-every 24 \
  --max-files 30
```

Writes CSV logs, rotating files every 24 hours or when they exceed 50 MB, keeping at most 30 files.

### Aggressive 1-second interval for debugging

```bash
sentinel --interval 1 --verbose --buffer-size 1
```

Collects every second, flushes every snapshot immediately, and enables debug-level tracing output.

## Configuration

| Flag | Description | Default |
|------|-------------|---------|
| `-i, --interval <SECONDS>` | Collection interval in seconds (min: 1) | `5` |
| `-f, --format <FORMAT>` | Output format: `json` or `csv` | `json` |
| `-o, --output-dir <PATH>` | Directory for log files | `./logs` |
| `--cpu-threshold <PERCENT>` | CPU alert threshold (0-100) | `90` |
| `--ram-threshold <PERCENT>` | RAM alert threshold (0-100) | `85` |
| `--alert-consecutive <N>` | Consecutive readings before alert fires | `3` |
| `--max-file-size <MB>` | Max log file size before rotation (min: 1) | `10` |
| `--rotate-every <HOURS>` | Time-based rotation interval (0 = disabled) | `0` |
| `--max-files <N>` | Max retained log files | `10` |
| `--webhook-url <URL>` | Webhook URL for alert notifications | disabled |
| `--buffer-size <N>` | Snapshots buffered before disk flush (min: 1) | `10` |
| `-v, --verbose` | Enable debug-level logging | `false` |

## Architecture

```
src/
├── main.rs          Entrypoint: CLI parsing, tracing setup, engine launch
├── lib.rs           Public module declarations
├── cli.rs           CLI argument definitions (clap derive API)
├── config.rs        Unified configuration struct with validation
├── models.rs        Data structs: SystemSnapshot, CpuMetrics, MemoryMetrics, etc.
├── engine.rs        Async main loop: orchestrates collection, alerting, persistence
├── collector/
│   ├── mod.rs       MetricCollector trait, SystemCollector orchestrator
│   ├── cpu.rs       CPU usage collection (global + per-core)
│   ├── memory.rs    RAM metrics collection
│   ├── disk.rs      Disk usage per mount point
│   └── network.rs   Network TX/RX per interface with delta tracking
├── alert/
│   ├── mod.rs       AlertEvaluator with consecutive-count debounce logic
│   ├── logger.rs    Alert dispatch via tracing::error
│   └── webhook.rs   Alert dispatch via HTTP POST with retry
└── persistence/
    ├── mod.rs       LogWriter trait and format-based factory
    ├── json_writer.rs  Buffered async JSONL writer
    ├── csv_writer.rs   Buffered async CSV writer with dynamic column layout
    └── rotation.rs     File rotation by size/time and retention enforcement
```

**Collector layer** uses `sysinfo` for cross-platform metric access. CPU collection runs inside `spawn_blocking` to avoid blocking the Tokio runtime. A shared `System` instance is reused across ticks to minimize overhead.

**Alert layer** tracks per-resource consecutive breach counts. Alerts fire once at threshold and stay silent until the value drops below and re-triggers (debounce). Webhook dispatch includes retry with backoff for transient HTTP errors.

**Persistence layer** buffers N snapshots in memory before flushing to disk. Rotation checks run after each flush. Old files beyond the retention limit are automatically deleted.

## License

[MIT](LICENSE)
