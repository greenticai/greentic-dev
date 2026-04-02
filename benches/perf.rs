use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

#[path = "../tests/support/perf_harness.rs"]
mod perf_harness;

fn bench_flow_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("flow_validation");

    for node_count in [10usize, 100usize] {
        let validator = perf_harness::build_validator();
        let document = perf_harness::build_document(node_count);

        group.throughput(Throughput::Elements(node_count as u64));
        group.bench_with_input(
            BenchmarkId::new("validate_document", node_count),
            &node_count,
            |b, _| {
                b.iter(|| {
                    validator
                        .validate_document(&document)
                        .expect("benchmark input should remain valid")
                })
            },
        );
    }

    group.finish();
}

fn bench_schema_compile_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("schema_validation_ab");

    for node_count in [10usize, 100usize] {
        group.throughput(Throughput::Elements(node_count as u64));
        group.bench_with_input(
            BenchmarkId::new("uncached_compile_per_node", node_count),
            &node_count,
            |b, &count| b.iter(|| perf_harness::run_schema_validation_loops_uncached(count, 1)),
        );
        group.bench_with_input(
            BenchmarkId::new("cached_single_compile", node_count),
            &node_count,
            |b, &count| b.iter(|| perf_harness::run_schema_validation_loops_cached(count, 1)),
        );
    }

    group.finish();
}

criterion_group!(benches, bench_flow_validation, bench_schema_compile_impact);
criterion_main!(benches);
