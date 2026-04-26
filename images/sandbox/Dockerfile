FROM alpine:3.20
RUN apk add --no-cache python3 py3-pip rust cargo gcc musl-dev nodejs
RUN pip3 install --break-system-packages uv
WORKDIR /workspace
ENTRYPOINT ["/bin/sh", "-c"]
