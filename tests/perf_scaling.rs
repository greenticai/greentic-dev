use std::thread;
use std::time::Duration;

#[path = "support/perf_harness.rs"]
mod perf_harness;

fn run_workload(threads: usize, node_count: usize, iterations: usize) -> Duration {
    let mut handles = Vec::with_capacity(threads);

    for _ in 0..threads {
        handles.push(thread::spawn(move || {
            perf_harness::run_validation_loops(node_count, iterations)
        }));
    }

    let mut longest = Duration::from_millis(0);
    for handle in handles {
        let elapsed = handle
            .join()
            .expect("scaling worker thread should complete without panic");
        if elapsed > longest {
            longest = elapsed;
        }
    }

    longest
}

fn median(values: &mut [Duration; 3]) -> Duration {
    values.sort_unstable();
    values[1]
}

#[test]
fn scaling_should_not_degrade_badly() {
    let mut one = [Duration::ZERO; 3];
    let mut four = [Duration::ZERO; 3];
    let mut eight = [Duration::ZERO; 3];

    for i in 0..3 {
        one[i] = run_workload(1, 100, 12);
        four[i] = run_workload(4, 100, 12);
        eight[i] = run_workload(8, 100, 12);
    }

    let t1 = median(&mut one);
    let t4 = median(&mut four);
    let t8 = median(&mut eight);
    println!("scaling medians: t1={t1:?}, t4={t4:?}, t8={t8:?}");

    assert!(
        t4 <= t1.mul_f64(3.0),
        "4-thread workload degraded unexpectedly: t1={:?}, t4={:?}",
        t1,
        t4
    );

    assert!(
        t8 <= t4.mul_f64(2.0),
        "8-thread workload degraded unexpectedly: t4={:?}, t8={:?}",
        t4,
        t8
    );
}
