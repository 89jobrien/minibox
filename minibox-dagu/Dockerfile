# minibox-dagu container image
#
# Multi-stage build: compile the Go executor plugin, then layer it into the
# official dagu base image.  Using the dagu base image (not alpine) ensures
# dagu's own entrypoint, configuration layout, and timezone data are present.
# Fixes GH #35 — previous drafts incorrectly used FROM alpine.

FROM golang:1.23-alpine AS plugin-builder
WORKDIR /src/minibox-dagu
COPY . .
RUN go build -o /tmp/mbx-dagu ./cmd/mbx-dagu

# Use the official dagu image — NOT alpine.
FROM ghcr.io/dagu-org/dagu:latest

RUN apk add --no-cache ca-certificates

# Install the compiled executor binary.
COPY --from=plugin-builder /tmp/mbx-dagu /usr/local/bin/mbx-dagu

# Register the minibox executor with dagu.
COPY dagu-config.yaml /etc/dagu/

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/dagu", "server", "--host", "0.0.0.0", "--port", "8080"]
