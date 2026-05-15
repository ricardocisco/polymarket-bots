FROM rust:1.86-slim AS builder

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --bin bot --bin backtest --bin settle_open_trades --bin sweep

FROM debian:bookworm-slim

ENV RUST_LOG=info

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /app/data /app/logs

COPY --from=builder /app/target/release/bot /usr/local/bin/bot
COPY --from=builder /app/target/release/backtest /usr/local/bin/backtest
COPY --from=builder /app/target/release/settle_open_trades /usr/local/bin/settle_open_trades
COPY --from=builder /app/target/release/sweep /usr/local/bin/sweep

CMD ["bot"]
