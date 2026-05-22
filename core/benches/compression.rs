#![allow(clippy::identity_op)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use shard_core::compression::Compression;

fn data_1mb() -> Vec<u8> {
    vec![0xAB; 1024 * 1024]
}

fn data_10mb() -> Vec<u8> {
    vec![0xCD; 10 * 1024 * 1024]
}

fn bench_compress(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression");

    let data_1mb = data_1mb();
    let data_10mb = data_10mb();

    for algo in ["zstd", "zlib", "none"] {
        group.bench_with_input(BenchmarkId::new("1mb_compress", algo), algo, |b, algo| {
            let compression: Compression = algo.parse().unwrap();
            b.iter(|| {
                let compressed = compression.compress(black_box(&data_1mb)).unwrap();
                black_box(compressed)
            });
        });
    }

    group.bench_function("10mb_zstd", |b| {
        let compression: Compression = "zstd".parse().unwrap();
        b.iter(|| {
            let compressed = compression.compress(black_box(&data_10mb)).unwrap();
            black_box(compressed)
        });
    });

    group.finish();
}

fn bench_decompress(c: &mut Criterion) {
    let mut group = c.benchmark_group("decompression");

    let data_1mb = data_1mb();

    for algo in ["zstd", "zlib", "none"] {
        let compression: Compression = algo.parse().unwrap();
        let compressed = compression.compress(&data_1mb).unwrap();
        group.bench_with_input(BenchmarkId::new("1mb_decompress", algo), algo, |b, algo| {
            let compression: Compression = algo.parse().unwrap();
            b.iter(|| {
                let decompressed = compression.decompress(black_box(&compressed)).unwrap();
                black_box(decompressed)
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_compress, bench_decompress);
criterion_main!(benches);
