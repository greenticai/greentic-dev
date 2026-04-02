use std::time::{Duration, Instant};

#[path = "support/perf_harness.rs"]
mod perf_harness;

#[test]
fn workload_should_finish_quickly() {
    let start = Instant::now();

    let elapsed = perf_harness::run_validation_loops(150, 25);

    let total_elapsed = start.elapsed();
    assert!(
        total_elapsed < Duration::from_secs(5),
        "validation workload exceeded timeout: workload={:?}, total={:?}",
        elapsed,
        total_elapsed
    );
}
