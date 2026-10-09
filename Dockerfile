# Megabase container image: the single `megabase` binary.
#
#   docker build -t megabase .
#   docker run --rm -p 8000:8000 megabase

# Digests verified against Docker Hub tags on 2026-10-09 (Scorecard Pinned-Dependencies).
# Renovate keeps tag+digest in sync (`docker` datasource, pinDigests).
FROM rust:1.89-slim-bookworm@sha256:d7fc7de78bb8c1469933aeecbf801314d30d7d6e9f0578bba4cfa285bfa37fe6 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY tools tools
COPY judge/harness judge/harness
RUN cargo build --release --locked -p megabase -p megabase-judge --bin megabase --bin megabase-healthcheck

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
COPY --from=build /src/target/release/megabase /usr/local/bin/megabase
COPY --from=build /src/target/release/megabase-healthcheck /usr/local/bin/megabase-healthcheck
ENV MEGABASE_HOST=0.0.0.0 MEGABASE_PORT=8000
EXPOSE 8000
USER 65534:65534
ENTRYPOINT ["/usr/local/bin/megabase"]
