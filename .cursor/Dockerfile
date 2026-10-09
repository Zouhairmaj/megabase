# Base image for Cursor Cloud Agents.
# Do not COPY the repository: Cursor checks out the requested revision separately.
# Digest verified against Docker Hub tag ubuntu:24.04 on 2026-10-09 (Scorecard Pinned-Dependencies).
FROM ubuntu:24.04@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55

ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_HOME=/home/ubuntu/.cargo \
    RUSTUP_HOME=/home/ubuntu/.rustup \
    PATH=/home/ubuntu/.cargo/bin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    CARGO_TERM_COLOR=always \
    RUST_BACKTRACE=1 \
    CARGO_INCREMENTAL=0

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        -o Dpkg::Options::="--force-confold" \
        -o Dpkg::Options::="--force-confdef" \
        ca-certificates \
        curl \
        git \
        gnupg \
        sudo \
        build-essential \
        pkg-config \
        libssl-dev \
        iptables \
        fuse-overlayfs \
        sysvinit-utils \
    && install -m 0755 -d /etc/apt/keyrings \
    && curl --retry 3 --retry-delay 5 -fsSL https://download.docker.com/linux/ubuntu/gpg \
        | gpg --dearmor -o /etc/apt/keyrings/docker.gpg \
    && chmod a+r /etc/apt/keyrings/docker.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo "$VERSION_CODENAME") stable" \
        > /etc/apt/sources.list.d/docker.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        -o Dpkg::Options::="--force-confold" \
        -o Dpkg::Options::="--force-confdef" \
        docker-ce=5:28.5.2-1~ubuntu.24.04~noble \
        docker-ce-cli=5:28.5.2-1~ubuntu.24.04~noble \
        containerd.io \
        docker-buildx-plugin \
        docker-compose-plugin \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /etc/docker \
    && printf '%s\n' '{' '  "storage-driver": "fuse-overlayfs"' '}' > /etc/docker/daemon.json \
    && update-alternatives --set iptables /usr/sbin/iptables-legacy \
    && update-alternatives --set ip6tables /usr/sbin/ip6tables-legacy \
    && id -u ubuntu >/dev/null 2>&1 || useradd -m -s /bin/bash ubuntu \
    && groupadd -f docker \
    && usermod -aG docker ubuntu \
    && usermod -aG sudo ubuntu \
    && echo "ubuntu ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/ubuntu \
    && chmod 0440 /etc/sudoers.d/ubuntu

USER ubuntu
WORKDIR /home/ubuntu
# rust-toolchain.toml (repo root) selects the exact channel after checkout.
# rustup-init 1.29.1 (rust-lang/rustup d95a37b6ab92cc1e455d1576039333c97ca3e2c5).
# cargo-binstall v1.25.2 (cargo-bins/cargo-binstall c06f6a0003f7c6165a00c89d1697731c9e8a5c72).
# Download, sha256-verify, then exec — no curl|sh (Scorecard downloadThenRun).
# rustup-init hashes match rust-lang/docker-rust stable/bookworm; cargo-binstall
# hashes match the GitHub release asset digests. Re-verified 2026-10-09.
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
        amd64) \
            rustArch='x86_64-unknown-linux-gnu'; \
            rustupSha256='dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71'; \
            binstallTriple='x86_64-unknown-linux-musl'; \
            binstallSha256='2b8687cb3297d335efdac1ee839fab47ac4c2b8f5f90b9e290aa7ba52f607803'; \
            ;; \
        arm64) \
            rustArch='aarch64-unknown-linux-gnu'; \
            rustupSha256='15f6e4ce9f583b929c996c91562bad6d4454f3281de858b02cdfdef615fac433'; \
            binstallTriple='aarch64-unknown-linux-musl'; \
            binstallSha256='f4d921de650ee4e04cb8ade2c1476987d21d55c23786d810a2913817c6134b94'; \
            ;; \
        *) echo >&2 "unsupported architecture: $arch"; exit 1 ;; \
    esac; \
    curl --proto '=https' --tlsv1.2 --retry 3 --retry-delay 5 -fsSL \
        "https://static.rust-lang.org/rustup/archive/1.29.1/${rustArch}/rustup-init" \
        -o rustup-init; \
    echo "${rustupSha256} *rustup-init" | sha256sum -c -; \
    chmod +x rustup-init; \
    ./rustup-init -y --default-toolchain stable --profile minimal \
        --component rustfmt --component clippy --component llvm-tools; \
    rm rustup-init; \
    curl --proto '=https' --tlsv1.2 --retry 3 --retry-delay 5 -fsSL \
        "https://github.com/cargo-bins/cargo-binstall/releases/download/v1.25.2/cargo-binstall-${binstallTriple}.tgz" \
        -o cargo-binstall.tgz; \
    echo "${binstallSha256} *cargo-binstall.tgz" | sha256sum -c -; \
    tar -xzf cargo-binstall.tgz -C /home/ubuntu/.cargo/bin cargo-binstall; \
    rm cargo-binstall.tgz; \
    cargo binstall -y just cargo-deny cargo-audit cargo-llvm-cov

USER root
RUN for b in rustup rustc cargo rustfmt cargo-clippy cargo-fmt clippy-driver \
        cargo-binstall just cargo-deny cargo-audit cargo-llvm-cov; do \
      if [ -x "/home/ubuntu/.cargo/bin/$b" ]; then \
        ln -sfn "/home/ubuntu/.cargo/bin/$b" "/usr/local/bin/$b"; \
      fi; \
    done
USER ubuntu
