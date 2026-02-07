# Start from the official Golang image for building

# ---- Build Stage ----
FROM golang:1.21-bullseye AS builder

WORKDIR /app

# Install OpenCV build dependencies and tools
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential pkg-config git cmake wget unzip \
    libgtk2.0-dev libavcodec-dev libavformat-dev libswscale-dev libv4l-dev \
    libxvidcore-dev libx264-dev libjpeg-dev libpng-dev libtiff-dev \
    libatlas-base-dev gfortran python3-dev ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Build OpenCV from source with Aruco module
ENV OPENCV_VERSION=4.5.5
WORKDIR /tmp
RUN wget -O opencv.zip https://github.com/opencv/opencv/archive/${OPENCV_VERSION}.zip && \
    unzip opencv.zip && \
    wget -O opencv_contrib.zip https://github.com/opencv/opencv_contrib/archive/${OPENCV_VERSION}.zip && \
    unzip opencv_contrib.zip && \
    cd opencv-${OPENCV_VERSION} && mkdir build && cd build && \
    cmake -D CMAKE_BUILD_TYPE=Release \
    -D CMAKE_INSTALL_PREFIX=/usr/local \
    -D OPENCV_EXTRA_MODULES_PATH=../../opencv_contrib-${OPENCV_VERSION}/modules \
    -D BUILD_EXAMPLES=OFF .. && \
    make -j$(nproc) && \
    make install && \
    ldconfig && \
    cd /app && rm -rf /tmp/*

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

# Install OpenCV runtime dependencies (minimal, OpenCV already installed)
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Copy the built binary from builder
COPY --from=builder /app/security-camera .

# Expose the port (adjust if your app uses a different port)
EXPOSE 8080

# Run the binary
CMD ["./security-camera"]
