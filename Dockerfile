# Start from the official Golang image for building

# ---- Build Stage ----
FROM golang:1.21-bullseye AS builder

WORKDIR /app

# Install OpenCV build dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential pkg-config \
    libopencv-dev \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Copy go mod and sum files
COPY go.mod .
COPY go.sum .

# Download dependencies
RUN go mod download

# Copy the source code
COPY . .

# Build the Go app (adjust the path to your main package if needed)
RUN go build -o security-camera ./cmd/server

# ---- Run Stage ----
FROM debian:bullseye-slim
WORKDIR /root/

# Install OpenCV runtime dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    libopencv-core-dev libopencv-imgproc-dev libopencv-highgui-dev libopencv-videoio-dev \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Copy the built binary from builder
COPY --from=builder /app/security-camera .

# Expose the port (adjust if your app uses a different port)
EXPOSE 8080

# Run the binary
CMD ["./security-camera"]
