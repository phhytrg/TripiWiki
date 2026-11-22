# Stage 1: Build the release binary
FROM rust:1-slim-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY templates ./templates

RUN cargo build --release

# Stage 2: Minimal runtime image
FROM debian:bookworm-slim
WORKDIR /app

COPY --from=builder /app/target/release/tripiwiki /app/tripiwiki
COPY content ./content
COPY static ./static

ENV PORT=3000
EXPOSE 3000

CMD ["/app/tripiwiki"]
