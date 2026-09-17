# syntax=docker/dockerfile:1.4
# BioRouter CLI and Server Docker Image
# Multi-stage build for minimal final image size

# Native helper is part of the CLI image even when no desktop is attached.
FROM --platform=$BUILDPLATFORM golang:1.26.8-bookworm AS computer-use-builder
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends python3 git ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY scripts/computer-use-runtime.py scripts/computer-use-runtime.py
COPY third_party/open-computer-use third_party/open-computer-use
RUN case "$TARGETARCH" in amd64) HELPER_TARGET=linux-x64 ;; arm64) HELPER_TARGET=linux-arm64 ;; *) echo "Unsupported native helper architecture: $TARGETARCH" >&2; exit 1 ;; esac && \
    python3 scripts/computer-use-runtime.py build "$HELPER_TARGET" && \
    cp -a "target/computer-use/$HELPER_TARGET" /computer-use

# Build stage
FROM rust:1.92-bookworm AS builder

# Install build dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    libdbus-1-dev \
    protobuf-compiler \
    libprotobuf-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /build

# Copy source code
COPY . .

# Build release binaries with optimizations
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV CARGO_PROFILE_RELEASE_LTO=true
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
ENV CARGO_PROFILE_RELEASE_OPT_LEVEL=z
ENV CARGO_PROFILE_RELEASE_STRIP=true
RUN cargo build --release --package biorouter-cli

# Runtime stage - minimal Debian
FROM debian:bookworm-slim@sha256:b1a741487078b369e78119849663d7f1a5341ef2768798f7b7406c4240f86aef

# Install only runtime dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    libdbus-1-3 \
    libxcb1 \
    python3 \
    python3-gi \
    gir1.2-atspi-2.0 \
    gir1.2-gtk-3.0 \
    at-spi2-core \
    curl \
    git \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Copy binary from builder
COPY --from=builder /build/target/release/biorouter /usr/local/bin/biorouter
COPY --from=computer-use-builder /computer-use /usr/libexec/biorouter/computer-use

# Create non-root user
RUN useradd -m -u 1000 -s /bin/bash biorouter && \
    mkdir -p /home/biorouter/.config/biorouter && \
    chown -R biorouter:biorouter /home/biorouter

# Set up environment
ENV PATH="/usr/local/bin:${PATH}"
ENV HOME="/home/biorouter"

# Switch to non-root user
USER biorouter
WORKDIR /home/biorouter

# Default to biorouter CLI
ENTRYPOINT ["/usr/local/bin/biorouter"]
CMD ["--help"]

# Labels for metadata
LABEL org.opencontainers.image.title="biorouter"
LABEL org.opencontainers.image.description="BioRouter CLI"
LABEL org.opencontainers.image.vendor="UCSF Baranzini Lab"
LABEL org.opencontainers.image.source="https://github.com/BaranziniLab/biorouter"
