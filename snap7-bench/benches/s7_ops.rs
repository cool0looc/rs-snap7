use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use snap7_bench_helper::connect_external;
use std::time::Duration;

fn bench_db_read(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let client = rt.block_on(connect_external());

    let mut group = c.benchmark_group("db_read");
    group.measurement_time(Duration::from_secs(10));

    for size in [1u16, 4, 8, 64, 240] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            b.to_async(&rt)
                .iter(|| async { client.db_read(1, 0, size).await.unwrap() });
        });
    }
    group.finish();
}

fn bench_db_write(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let client = rt.block_on(connect_external());

    let mut group = c.benchmark_group("db_write");
    group.measurement_time(Duration::from_secs(10));

    for size in [1usize, 4, 8, 64, 240] {
        let payload = vec![0xABu8; size];
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &payload, |b, payload| {
            b.to_async(&rt)
                .iter(|| async { client.db_write(2, 0, payload).await.unwrap() });
        });
    }
    group.finish();
}

fn bench_roundtrip(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let client = rt.block_on(connect_external());

    let mut group = c.benchmark_group("roundtrip_write_read");
    group.measurement_time(Duration::from_secs(10));

    let payload = vec![0xDEu8; 8];
    group.bench_function("8_bytes", |b| {
        b.to_async(&rt).iter(|| async {
            client.db_write(3, 0, &payload).await.unwrap();
            client.db_read(3, 0, 8).await.unwrap()
        });
    });
    group.finish();
}

criterion_group!(benches, bench_db_read, bench_db_write, bench_roundtrip);
criterion_main!(benches);
