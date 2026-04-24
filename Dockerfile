FROM rust:1.83-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p polybot-core

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/polybot-core /app/polybot-core
COPY --from=builder /app/config.toml /app/config.toml
EXPOSE 8080 8081
CMD ["/app/polybot-core"]
