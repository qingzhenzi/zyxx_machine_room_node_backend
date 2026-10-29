# syntax=docker/dockerfile:1

# 内网/镜像站可覆盖：docker build --build-arg RUST_IMAGE=... --build-arg RUNTIME_IMAGE=...
ARG RUST_IMAGE=rust:1.97-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

FROM ${RUST_IMAGE} AS builder
WORKDIR /app

# 先用空 crate 编译依赖，利用 Docker 层缓存加速重复构建。
# 所有 workspace 成员都要放占位 manifest/lib，否则 cargo 解析或构建失败。
COPY Cargo.toml Cargo.lock ./
COPY crates/api/Cargo.toml crates/api/
COPY crates/auth/Cargo.toml crates/auth/
COPY crates/cache/Cargo.toml crates/cache/
COPY crates/db/Cargo.toml crates/db/
COPY crates/domain/Cargo.toml crates/domain/
RUN mkdir -p crates/api/src crates/auth/src crates/cache/src crates/db/src crates/domain/src \
    && printf 'fn main() {}\n' > crates/api/src/main.rs \
    && printf '\n' > crates/api/src/lib.rs \
    && printf '\n' > crates/auth/src/lib.rs \
    && printf '\n' > crates/cache/src/lib.rs \
    && printf '\n' > crates/db/src/lib.rs \
    && printf '\n' > crates/domain/src/lib.rs \
    && cargo build --release --locked -p qe-api \
    && rm -rf crates/api/src crates/auth/src crates/cache/src crates/db/src crates/domain/src

COPY . .
RUN find crates -name '*.rs' -exec touch {} + \
    && cargo build --release --locked -p qe-api

FROM ${RUNTIME_IMAGE} AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --user-group --create-home app

COPY --from=builder /app/target/release/qe-api /usr/local/bin/qe-api

ENV RUST_LOG=info \
    BIND_ADDR=0.0.0.0:8080

USER app
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/qe-api"]
