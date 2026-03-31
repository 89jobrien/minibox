# Build stage
FROM golang:1.22-alpine AS builder
WORKDIR /src
COPY go.mod ./
COPY . .
RUN CGO_ENABLED=0 GOOS=linux go build -ldflags="-s -w" -o /mbx-dagu ./cmd/mbx-dagu

# Final image: dagu + mbx-dagu binary
FROM ghcr.io/dagu-org/dagu:latest
COPY --from=builder /mbx-dagu /usr/local/bin/mbx-dagu

# Default: mbxctl on localhost:9999 (use --mbxctl or MBXCTL_URL to override)
ENV MBXCTL_URL=http://localhost:9999
