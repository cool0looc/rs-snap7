use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use snap7_bench_helper::connect_external;
use std::time::Duration;
use tokio::runtime::Builder;

fn bench_db_read_sync(c: &mut Criterion) {
    // Single-threaded runtime — one block_on per call, no task scheduler overhead.
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let client = rt.block_on(connect_external());

    let mut group = c.benchmark_group("db_read_sync");
    group.measurement_time(Duration::from_secs(10));

    for size in [1u16, 4, 8, 64, 240] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            b.iter(|| rt.block_on(async { client.db_read(1, 0, size).await.unwrap() }));
        });
    }
    group.finish();
}

fn bench_db_write_sync(c: &mut Criterion) {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let client = rt.block_on(connect_external());

    let mut group = c.benchmark_group("db_write_sync");
    group.measurement_time(Duration::from_secs(10));

    for size in [1usize, 4, 8, 64, 240] {
        let payload = vec![0xABu8; size];
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &payload, |b, payload| {
            b.iter(|| rt.block_on(async { client.db_write(2, 0, payload).await.unwrap() }));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_db_read_sync, bench_db_write_sync);
criterion_main!(benches);
