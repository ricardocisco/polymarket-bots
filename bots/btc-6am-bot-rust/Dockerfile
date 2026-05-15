FROM rust:1-slim-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked --bins

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/bot /usr/local/bin/bot
COPY --from=builder /app/target/release/backtest /usr/local/bin/backtest
COPY --from=builder /app/target/release/settle_open_trades /usr/local/bin/settle_open_trades
COPY --from=builder /app/target/release/sweep /usr/local/bin/sweep

RUN mkdir -p data

CMD ["bot"]
