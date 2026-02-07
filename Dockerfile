# Start from the official Golang image for building
FROM golang:1.21-alpine AS builder

WORKDIR /app

# Copy go mod and sum files
COPY go.mod .
COPY go.sum .

# Download dependencies
RUN go mod download

# Copy the source code
COPY . .

# Build the Go app (adjust the path to your main package if needed)
RUN go build -o security-camera ./cmd/server

# Start a minimal image for running
FROM alpine:latest
WORKDIR /root/

# Install CA certificates (for HTTPS, Mongo, etc.)
RUN apk --no-cache add ca-certificates

# Copy the built binary from builder
COPY --from=builder /app/security-camera .

# Copy any static/config files if needed
# COPY ./config ./config

# Expose the port (adjust if your app uses a different port)
EXPOSE 8080

# Set environment variables if needed
# ENV MONGO_URI=mongodb://mongo:27017/security-camera

# Run the binary
CMD ["./security-camera"]
