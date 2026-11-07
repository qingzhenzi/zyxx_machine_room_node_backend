# syntax=docker/dockerfile:1

# 内网/镜像站可覆盖：docker build --build-arg RUST_IMAGE=... --build-arg RUNTIME_IMAGE=...
ARG RUST_IMAGE=rust:1.97-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

FROM ${RUST_IMAGE} AS builder
WORKDIR /app

# 先用空 crate 编译依赖，利用 Docker 层缓存加速重复构建。
# 所有 workspace 成员（7 个）都要放占位 manifest + 占位 src，否则 cargo 解析
# workspace / 路径依赖时会直接失败（历史踩坑：漏了 storage、device-pki）。
COPY Cargo.toml Cargo.lock ./
COPY crates/api/Cargo.toml crates/api/
COPY crates/auth/Cargo.toml crates/auth/
COPY crates/cache/Cargo.toml crates/cache/
COPY crates/db/Cargo.toml crates/db/
COPY crates/domain/Cargo.toml crates/domain/
COPY crates/storage/Cargo.toml crates/storage/
COPY crates/device-pki/Cargo.toml crates/device-pki/
RUN mkdir -p crates/api/src crates/auth/src crates/cache/src crates/db/src \
             crates/domain/src crates/storage/src crates/device-pki/src \
    && printf 'fn main() {}\n' > crates/api/src/main.rs \
    && printf 'fn main() {}\n' > crates/device-pki/src/main.rs \
    && printf '\n' > crates/api/src/lib.rs \
    && printf '\n' > crates/auth/src/lib.rs \
    && printf '\n' > crates/cache/src/lib.rs \
    && printf '\n' > crates/db/src/lib.rs \
    && printf '\n' > crates/domain/src/lib.rs \
    && printf '\n' > crates/storage/src/lib.rs \
    && printf '\n' > crates/device-pki/src/lib.rs \
    && cargo build --release --locked -p qe-api -p qe-device-pki \
    && rm -rf crates/api/src crates/auth/src crates/cache/src crates/db/src \
              crates/domain/src crates/storage/src crates/device-pki/src

COPY . .
RUN find crates -name '*.rs' -exec touch {} + \
    && cargo build --release --locked -p qe-api -p qe-device-pki

FROM ${RUNTIME_IMAGE} AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --user-group --create-home app

# 单镜像双二进制：默认入口 qe-api；device-pki 服务用 compose entrypoint 覆盖。
COPY --from=builder /app/target/release/qe-api /usr/local/bin/qe-api
COPY --from=builder /app/target/release/qe-device-pki /usr/local/bin/qe-device-pki

ENV RUST_LOG=info \
    BIND_ADDR=0.0.0.0:8080

USER app
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/qe-api"]
