use std::process::Command;

fn sentinel_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sentinel"))
}

#[test]
fn help_flag_exits_successfully() {
    let output = sentinel_bin()
        .arg("--help")
        .output()
        .expect("should execute binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("sentinel"),
        "help output should mention the program name"
    );
}

#[test]
fn version_flag_exits_successfully() {
    let output = sentinel_bin()
        .arg("--version")
        .output()
        .expect("should execute binary");

    assert!(output.status.success());
}

#[test]
fn rejects_interval_zero() {
    let output = sentinel_bin()
        .args(["--interval", "0"])
        .output()
        .expect("should execute binary");

    assert!(
        !output.status.success(),
        "interval 0 should be rejected by the parser"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("interval"),
        "error message should reference interval"
    );
}

#[test]
fn rejects_invalid_format() {
    let output = sentinel_bin()
        .args(["--format", "xml"])
        .output()
        .expect("should execute binary");

    assert!(
        !output.status.success(),
        "unsupported format should be rejected"
    );
}

#[test]
fn rejects_cpu_threshold_above_100() {
    let output = sentinel_bin()
        .args(["--cpu-threshold", "101"])
        .output()
        .expect("should execute binary");

    assert!(!output.status.success());
}

#[test]
fn rejects_ram_threshold_negative() {
    let output = sentinel_bin()
        .args(["--ram-threshold", "-1"])
        .output()
        .expect("should execute binary");

    assert!(!output.status.success());
}

#[test]
fn rejects_buffer_size_zero() {
    let output = sentinel_bin()
        .args(["--buffer-size", "0"])
        .output()
        .expect("should execute binary");

    assert!(!output.status.success());
}

#[test]
fn rejects_max_file_size_zero() {
    let output = sentinel_bin()
        .args(["--max-file-size", "0"])
        .output()
        .expect("should execute binary");

    assert!(!output.status.success());
}
