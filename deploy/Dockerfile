# SPDX-License-Identifier: AGPL-3.0-only
# Control plane image. Build from the repository root:
#   docker build -f deploy/Dockerfile -t bennghe/controlplane:dev .

FROM node:26-alpine AS web
RUN npm install --global pnpm@12
WORKDIR /src/web
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web ./
RUN pnpm build

FROM rust:alpine AS rust
RUN apk add --no-cache musl-dev
WORKDIR /src
# The compiler setting file asks for components this image does not need.
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY proto proto
COPY migrations migrations
COPY .sqlx .sqlx
COPY --from=web /src/web/dist web/dist
ENV SQLX_OFFLINE=true
RUN cargo build --release -p bn-cp \
 && cp /src/target/release/controlplane /controlplane

FROM alpine:3.22
RUN apk add --no-cache ca-certificates \
 && addgroup -S -g 10001 bennghe && adduser -S -u 10001 -G bennghe -h /data bennghe \
 && mkdir -p /data && chown bennghe:bennghe /data && chmod 700 /data
COPY --from=rust /controlplane /usr/local/bin/controlplane
USER bennghe
ENV BN_DATA_DIR=/data
VOLUME /data
EXPOSE 80 443
ENTRYPOINT ["controlplane"]
CMD ["serve"]
