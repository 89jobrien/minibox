# Stage 1: Build miniboxd and mbx for musl (static binaries)
FROM rust:1.85-alpine AS builder

RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static

WORKDIR /build
COPY . .

RUN cargo build --release -p miniboxd -p mbx

# Stage 2: Minimal runtime image
FROM alpine:3.21

# proot is the unprivileged container runtime used by the GKE adapter
RUN apk add --no-cache proot

# Non-root user — GKE adapter does not require root
RUN adduser -D -h /home/minibox minibox

COPY --from=builder /build/target/release/miniboxd /usr/local/bin/miniboxd
COPY --from=builder /build/target/release/mbx /usr/local/bin/mbx

RUN mkdir -p /run/minibox /var/lib/minibox \
    && chown minibox:minibox /run/minibox /var/lib/minibox

ENV MINIBOX_ADAPTER=gke
ENV RUST_LOG=info

VOLUME ["/run/minibox", "/var/lib/minibox"]

USER minibox

ENTRYPOINT ["/usr/local/bin/miniboxd"]

# Stage 3: Test image — keeps toolchain + source + nextest for CI
FROM builder AS test

RUN cargo install cargo-nextest --locked

ENV RUST_LOG=info

ENTRYPOINT ["cargo"]
CMD ["nextest", "run", "--workspace", "--lib"]
