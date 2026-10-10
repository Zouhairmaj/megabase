//! Trivial gateway benches so the Bencher pipeline has a real Criterion signal.
//!
//! These are smoke measurements (liveness + one Kong-prefixed route), not a
//! published performance claim. They call `create_router`, so the production
//! HTTP layers are inside the sample. CI parses the default Criterion text
//! output with the `rust_criterion` adapter (`cargo bench --bench health`).
//! Bencher stores that run's point estimate. Fifty samples over three
//! seconds keep one internal outlier from moving the estimate. The alert
//! is a t-test on recent runs (`docs/decisions/0027-bencher-ttest-latency.md`).

use std::hint::black_box;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_config() -> Criterion {
    // A restored `target/criterion/.../base` directory without `sample.json`
    // makes Criterion log "Failed to access file" and exit the comparison.
    // Bencher reads the timing lines, not that on-disk baseline.
    let dir = std::env::temp_dir().join(format!("megabase-criterion-{}", std::process::id()));
    Criterion::default().output_directory(&dir)
}
use megabase_server::{create_router, HEALTH_PATH};
use tokio::runtime::Runtime;
use tower::ServiceExt;

fn gateway(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("gateway");
    // Bencher keeps one estimate per run. A short sample lets a single
    // outlier move that estimate even when the function is stable.
    group.sample_size(50);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));

    let health_router = create_router();
    group.bench_function("GET /_megabase/health", |b| {
        b.iter(|| {
            rt.block_on(async {
                let response = health_router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(HEALTH_PATH)
                            .body(Body::empty())
                            .expect("health request"),
                    )
                    .await
                    .expect("health oneshot");
                assert_eq!(response.status(), StatusCode::OK);
                black_box(response.status())
            })
        });
    });

    let rest_router = create_router();
    group.bench_function("GET /rest/v1/todos", |b| {
        b.iter(|| {
            rt.block_on(async {
                let response = rest_router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri("/rest/v1/todos")
                            .body(Body::empty())
                            .expect("rest request"),
                    )
                    .await
                    .expect("rest oneshot");
                assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
                black_box(response.status())
            })
        });
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = bench_config();
    targets = gateway
}
criterion_main!(benches);
