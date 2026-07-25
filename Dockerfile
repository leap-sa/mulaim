FROM rust:1.89-bookworm AS builder
WORKDIR /app

# Cache dependencies separately from source changes.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src target/release/mulaim target/release/deps/mulaim-*

COPY assets ./assets
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
LABEL org.opencontainers.image.title="Mulaim ملائم" \
      org.opencontainers.image.description="Local LLM fit checker — CLI, bilingual (EN/AR) web app, and JSON API" \
      org.opencontainers.image.vendor="LEAP RD&O واثب" \
      org.opencontainers.image.url="https://leap.sa" \
      org.opencontainers.image.licenses="MIT"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/mulaim /usr/local/bin/mulaim
ENV PORT=8080
EXPOSE 8080
ENTRYPOINT ["mulaim"]
CMD ["serve"]
