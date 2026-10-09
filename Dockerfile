# Megabase container image: the single `megabase` binary.
#
#   docker build -t megabase .
#   docker run --rm -p 8000:8000 megabase

FROM rust:1.89-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY tools tools
COPY judge/harness judge/harness
RUN cargo build --release --locked -p megabase -p megabase-judge --bin megabase --bin megabase-healthcheck

FROM debian:bookworm-slim
COPY --from=build /src/target/release/megabase /usr/local/bin/megabase
COPY --from=build /src/target/release/megabase-healthcheck /usr/local/bin/megabase-healthcheck
ENV MEGABASE_HOST=0.0.0.0 MEGABASE_PORT=8000
EXPOSE 8000
USER 65534:65534
ENTRYPOINT ["/usr/local/bin/megabase"]
