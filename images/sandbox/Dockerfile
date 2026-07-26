FROM alpine:3.20
RUN apk add --no-cache python3 py3-pip rust cargo gcc musl-dev nodejs
RUN pip3 install --break-system-packages uv
RUN cargo install rust-script
RUN adduser -D -h /workspace sandbox
WORKDIR /workspace
USER sandbox
ENTRYPOINT ["/bin/sh", "-c"]
