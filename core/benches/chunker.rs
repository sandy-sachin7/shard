#![allow(clippy::identity_op)]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use shard_core::chunker::Chunker;
use std::io::Cursor;

fn bench_fixed_chunking(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunker/fixed");

    for size in [64 * 1024, 256 * 1024, 1 * 1024 * 1024, 4 * 1024 * 1024] {
        group.bench_with_input(BenchmarkId::new("1mb", size), &size, |b, &size| {
            b.iter(|| {
                let data = vec![0xABu8; 1024 * 1024];
                let reader = Cursor::new(data);
                let mut chunker = Chunker::new_fixed(Box::new(reader), size as u64);
                let mut count = 0usize;
                while chunker.next_chunk().unwrap().is_some() {
                    count += 1;
                }
                count
            });
        });
    }

    group.bench_function("10mb_4mb_chunks", |b| {
        b.iter(|| {
            let data = vec![0xCBu8; 10 * 1024 * 1024];
            let reader = Cursor::new(data);
            let mut chunker = Chunker::new_fixed(Box::new(reader), 4 * 1024 * 1024);
            let mut count = 0usize;
            while chunker.next_chunk().unwrap().is_some() {
                count += 1;
            }
            count
        });
    });

    group.finish();
}

fn bench_fixed_synthetic(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunker/fixed/synthetic");
    for size in [64 * 1024, 256 * 1024] {
        group.bench_with_input(BenchmarkId::new("synthetic", size), &size, |b, &size| {
            b.iter(|| {
                let mut data = Vec::with_capacity(512 * 1024);
                for i in 0u8..50 {
                    data.extend_from_slice(&[i; 10 * 1024]);
                    data.extend_from_slice(&[i.wrapping_add(1); 10 * 1024]);
                }
                let reader = Cursor::new(data);
                let mut chunker = Chunker::new_fixed(Box::new(reader), size as u64);
                let mut count = 0usize;
                while chunker.next_chunk().unwrap().is_some() {
                    count += 1;
                }
                count
            });
        });
    }
    group.finish();
}

fn bench_rabin_chunking(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunker/rabin");

    for size in [256 * 1024, 1 * 1024 * 1024, 4 * 1024 * 1024] {
        group.bench_with_input(BenchmarkId::new("1mb", size), &size, |b, &size| {
            b.iter(|| {
                let data = vec![0xABu8; 1024 * 1024];
                let reader = Cursor::new(data);
                let mut chunker = Chunker::new_rabin(
                    Box::new(reader),
                    size as u64 / 4,
                    size as u64,
                    size as u64 * 2,
                );
                let mut count = 0usize;
                while chunker.next_chunk().unwrap().is_some() {
                    count += 1;
                }
                count
            });
        });
    }

    group.bench_function("10mb_avg4mb", |b| {
        b.iter(|| {
            let data = vec![0xCDu8; 10 * 1024 * 1024];
            let reader = Cursor::new(data);
            let mut chunker = Chunker::new_rabin(
                Box::new(reader),
                1 * 1024 * 1024,
                4 * 1024 * 1024,
                8 * 1024 * 1024,
            );
            let mut count = 0usize;
            while chunker.next_chunk().unwrap().is_some() {
                count += 1;
            }
            count
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_fixed_chunking,
    bench_rabin_chunking,
    bench_fixed_synthetic
);
criterion_main!(benches);
