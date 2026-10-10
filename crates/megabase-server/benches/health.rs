//! Trivial gateway benches so the Bencher pipeline has a real Criterion signal.
//!
//! These are smoke measurements (liveness + one Kong-prefixed route), not a
//! published performance claim. They call `create_router`, so the production
//! HTTP layers are inside the sample. CI parses the default Criterion text
//! output with the `rust_criterion` adapter (`cargo bench --bench health`).

use std::hint::black_box;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use criterion::{criterion_group, criterion_main, Criterion};
use megabase_server::{create_router, HEALTH_PATH};
use tokio::runtime::Runtime;
use tower::ServiceExt;

fn gateway(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("gateway");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(1));

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

criterion_group!(benches, gateway);
criterion_main!(benches);
