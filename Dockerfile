# Megabase container image: the single `megabase` binary.
#
#   docker build -t megabase .
#   docker run --rm -p 8000:8000 megabase

# Official Docker Hub tags and digests (verified 2026-10-09), pulled via
# Amazon ECR Public (`public.ecr.aws/docker/library`) so CI is not
# rate-limited by Hub 429s. Scorecard Pinned-Dependencies: tag+digest.
# Renovate keeps tag+digest in sync (`docker` datasource, pinDigests).
FROM public.ecr.aws/docker/library/rust:1.89-slim-bookworm@sha256:d7fc7de78bb8c1469933aeecbf801314d30d7d6e9f0578bba4cfa285bfa37fe6 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
# Offline sqlx query metadata. `.cargo/config.toml` sets SQLX_OFFLINE; the
# ENV keeps the build offline even if that file is not consulted.
COPY .cargo .cargo
COPY .sqlx .sqlx
COPY crates crates
COPY tools tools
COPY judge/harness judge/harness
# mold speeds the link. The package is installed in this stage, so the flag
# is only set where the linker exists.
RUN apt-get update \
    && apt-get install -y --no-install-recommends mold \
    && rm -rf /var/lib/apt/lists/*
ENV SQLX_OFFLINE=true
ENV RUSTFLAGS="-C link-arg=-fuse-ld=mold"
RUN cargo build --release --locked -p megabase -p megabase-judge --bin megabase --bin megabase-healthcheck

FROM public.ecr.aws/docker/library/debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
COPY --from=build /src/target/release/megabase /usr/local/bin/megabase
COPY --from=build /src/target/release/megabase-healthcheck /usr/local/bin/megabase-healthcheck
ENV MEGABASE_HOST=0.0.0.0 MEGABASE_PORT=8000
EXPOSE 8000
USER 65534:65534
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/megabase-healthcheck"]
ENTRYPOINT ["/usr/local/bin/megabase"]
