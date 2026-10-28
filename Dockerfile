# syntax=docker/dockerfile:1

# 内网/镜像站可覆盖：docker build --build-arg RUST_IMAGE=... --build-arg RUNTIME_IMAGE=...
ARG RUST_IMAGE=rust:1.97-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

FROM ${RUST_IMAGE} AS builder
WORKDIR /app

# 先用空 crate 编译依赖，利用 Docker 层缓存加速重复构建。
# workspace 成员（crates/auth）也要放占位 manifest，否则 cargo 解析失败。
COPY Cargo.toml Cargo.lock ./
COPY crates/auth/Cargo.toml crates/auth/
RUN mkdir -p src crates/auth/src \
    && printf 'fn main() {}\n' > src/main.rs \
    && printf '\n' > src/lib.rs \
    && printf '\n' > crates/auth/src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src crates/auth/src

COPY . .
RUN find src crates -name '*.rs' -exec touch {} + \
    && cargo build --release --locked

FROM ${RUNTIME_IMAGE} AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --user-group --create-home app

COPY --from=builder /app/target/release/backend /usr/local/bin/backend

ENV RUST_LOG=info \
    BIND_ADDR=0.0.0.0:8080

USER app
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/backend"]
