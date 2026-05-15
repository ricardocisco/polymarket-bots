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
COPY --from=builder /app/target/release/setup /usr/local/bin/setup
COPY --from=builder /app/target/release/backtest /usr/local/bin/backtest
COPY --from=builder /app/target/release/sim_monitor /usr/local/bin/sim_monitor
COPY --from=builder /app/target/release/monitor /usr/local/bin/monitor

RUN mkdir -p data

CMD ["bot"]
