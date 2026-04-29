use sentinel::collector::SystemCollector;

#[tokio::test]
async fn collects_complete_snapshot_with_plausible_values() {
    let mut collector = SystemCollector::new();

    let snapshot = collector
        .collect()
        .await
        .expect("system collection should succeed on any real machine");

    assert!(
        !snapshot.timestamp.is_empty(),
        "timestamp should be populated"
    );
    assert!(
        snapshot.timestamp.ends_with('Z'),
        "timestamp should be UTC with Z suffix"
    );

    assert!(
        (0.0..=100.0).contains(&snapshot.cpu.global_usage),
        "CPU usage {:.1}% should be within 0-100",
        snapshot.cpu.global_usage
    );
    assert!(
        !snapshot.cpu.per_core.is_empty(),
        "should detect at least one CPU core"
    );
    for (i, core_usage) in snapshot.cpu.per_core.iter().enumerate() {
        assert!(
            (0.0..=100.0).contains(core_usage),
            "core {} usage {:.1}% should be within 0-100",
            i,
            core_usage
        );
    }

    assert!(
        snapshot.memory.total_bytes > 0,
        "total memory should be positive"
    );
    assert!(
        snapshot.memory.used_bytes <= snapshot.memory.total_bytes,
        "used memory ({}) should not exceed total ({})",
        snapshot.memory.used_bytes,
        snapshot.memory.total_bytes
    );
    assert!(
        (0.0..=100.0).contains(&snapshot.memory.usage_percent),
        "memory usage {:.1}% should be within 0-100",
        snapshot.memory.usage_percent
    );

    assert!(
        !snapshot.disks.is_empty(),
        "should detect at least one disk"
    );
    for disk in &snapshot.disks {
        assert!(
            !disk.mount_point.is_empty(),
            "disk mount point should be set"
        );
        assert!(disk.total_bytes > 0, "disk total bytes should be positive");
        assert!(
            (0.0..=100.0).contains(&disk.usage_percent),
            "disk {} usage {:.1}% should be within 0-100",
            disk.mount_point,
            disk.usage_percent
        );
    }

    for iface in &snapshot.network {
        assert!(
            !iface.interface.is_empty(),
            "network interface name should be set"
        );
    }
}

#[tokio::test]
async fn consecutive_collections_produce_valid_ordered_timestamps() {
    let mut collector = SystemCollector::new();

    let first = collector.collect().await.expect("first collection");
    let second = collector.collect().await.expect("second collection");

    assert!(
        second.timestamp >= first.timestamp,
        "second timestamp ({}) should be >= first ({})",
        second.timestamp,
        first.timestamp
    );
}
