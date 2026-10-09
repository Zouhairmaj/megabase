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
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain stable --profile minimal \
            --component rustfmt --component clippy --component llvm-tools \
    && curl -L --proto '=https' --tlsv1.2 -sSf \
        https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash \
    && cargo binstall -y just cargo-deny cargo-audit cargo-llvm-cov

USER root
RUN for b in rustup rustc cargo rustfmt cargo-clippy cargo-fmt clippy-driver \
        cargo-binstall just cargo-deny cargo-audit cargo-llvm-cov; do \
      if [ -x "/home/ubuntu/.cargo/bin/$b" ]; then \
        ln -sfn "/home/ubuntu/.cargo/bin/$b" "/usr/local/bin/$b"; \
      fi; \
    done
USER ubuntu
