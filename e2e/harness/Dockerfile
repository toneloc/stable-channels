# Builds the sc-e2e-harness binary (counterparty node + miner + price mock).
FROM rust:1.82-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/sc-e2e-harness /usr/local/bin/sc-e2e-harness
EXPOSE 9736 9737
CMD ["/usr/local/bin/sc-e2e-harness"]
