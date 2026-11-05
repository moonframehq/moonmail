# Build only the fixture daemon, using the same pinned compiler as the workspace.
FROM rust:1.99.0-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends cmake && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo build --locked --release -p moonmail-test-maild

FROM debian:bookworm-slim
RUN useradd --uid 10001 --create-home fixture && mkdir /state && chown fixture:fixture /state
COPY --from=build /build/target/release/moonmail-test-maild /usr/local/bin/moonmail-test-maild
USER fixture
EXPOSE 1993 1465 1143 1587
ENTRYPOINT ["moonmail-test-maild"]
CMD ["--bind", "0.0.0.0", "--state", "/state"]
