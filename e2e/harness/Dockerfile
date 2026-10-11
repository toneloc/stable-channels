# syntax=docker/dockerfile:1.7
# Builds the sc-e2e-harness binary (counterparty node + miner + price mock).
FROM rust:1-bookworm AS build
ARG CARGO_BUILD_JOBS=1
ARG CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=${CARGO_PROFILE_RELEASE_CODEGEN_UNITS}
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN --mount=type=cache,id=harness-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=harness-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=harness-target,target=/src/target \
    cargo build --release -j "$CARGO_BUILD_JOBS" \
    && cp /src/target/release/sc-e2e-harness /tmp/sc-e2e-harness

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /tmp/sc-e2e-harness /usr/local/bin/sc-e2e-harness
EXPOSE 9736 9737
CMD ["/usr/local/bin/sc-e2e-harness"]
