# Multi-stage build for Vanity Server
# Optimized for Railway deployment

# Stage 1: Build stage
FROM rust:1.84-slim as builder

# Install system dependencies for building Rust crates
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /app

# Copy all necessary files for the build
COPY . ./

# Set environment variables to use system OpenSSL
ENV OPENSSL_NO_VENDOR=1
ENV PKG_CONFIG_ALLOW_CROSS=1

# Build the vanity server binary in release mode with server feature
RUN cargo build --release --features server --bin vanity

# Stage 2: Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create a non-root user
RUN useradd -m -u 1001 vanity

# Set working directory
WORKDIR /app

# Copy the binary from builder stage
COPY --from=builder /app/target/release/vanity /app/vanity

# Change ownership to the vanity user
RUN chown -R vanity:vanity /app

# Switch to non-root user
USER vanity

# Expose server port
EXPOSE 8080

# Set default environment variables
ENV RUST_LOG=info
ENV RUST_BACKTRACE=1
ENV VANITY_PORT=8080

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=40s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Start the vanity server
ENTRYPOINT ["./vanity", "server"]
