# syntax=docker/dockerfile:1

# ---------- builder: wasm frontend + release backend ----------
FROM rust:1-bookworm AS builder
WORKDIR /app
ENV CARGO_TERM_COLOR=never

# Cargo.lock first: the wasm-bindgen CLI must match the version the code resolves to.
COPY Cargo.toml Cargo.lock ./

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    bash -c ' \
      rustup target add wasm32-unknown-unknown && \
      cargo install --locked trunk && \
      VER=$(grep -A1 "name = \"wasm-bindgen\"" Cargo.lock | sed -n "s/.*version = \"\([^\"]*\)\".*/\1/p" | head -1) && \
      echo "wasm-bindgen-cli $VER" && \
      cargo install --locked wasm-bindgen-cli --version "$VER"'

COPY shared shared
COPY backend backend
COPY frontend frontend

# Frontend (trunk, wasm) and backend release build; target dirs ride on cache mounts
# so recompiles after source changes stay incremental. Artifacts are staged to /out
# because cache-mount contents do not persist into the image layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/app/frontend/target \
    bash -c 'cd frontend && trunk build --release && cd .. && cargo build --release -p backend \
             && mkdir -p /out \
             && cp target/release/jellydownloader /out/ \
             && cp -r frontend/dist /out/dist \
             && ls -la /out /out/dist'

# ---------- runtime ----------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 1000 jdl

WORKDIR /app
COPY --from=builder /out/jellydownloader /usr/local/bin/jellydownloader
COPY --from=builder /out/dist /app/frontend/dist

RUN mkdir -p /app/downloads /app/data && chown -R jdl:jdl /app
VOLUME ["/app/downloads", "/app/data"]
USER jdl

EXPOSE 3001
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -sf http://127.0.0.1:3001/api/health >/dev/null || exit 1

ENTRYPOINT ["/usr/local/bin/jellydownloader"]
