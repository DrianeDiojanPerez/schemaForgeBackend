# ──── Base ──────────────────────────────────
FROM rust:1.93-bookworm AS base

WORKDIR /app

ENV CARGO_TERM_COLOR=always

# The build script compiles the proto contract, so protoc has to be here
# before anything is built.
RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Dependency layer: only the manifests, so a source change does not rebuild
# the whole dependency tree. The build script and the contract come with them,
# since cargo runs the script before it will build anything. The toolchain
# file comes too, so rustup installs it here instead of when the container
# starts, where it would need the network on every fresh run.
COPY Cargo.toml Cargo.lock build.rs rust-toolchain.toml ./
COPY proto ./proto
RUN mkdir -p src && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src

# ──── Dev ──────────────────────────────────
FROM base AS development

ARG USERNAME=appuser

RUN groupadd --force -g 1000 $USERNAME \
    && useradd -ms /bin/bash --no-user-group -g 1000 -u 1000 $USERNAME \
    && apt-get update \
    && apt-get install -y --no-install-recommends tzdata \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-watch --locked

COPY . .

# The cargo home is populated by root in the base stage. The named volumes
# mounted over these paths inherit whatever the image owns, so both have to be
# handed over before the container drops to $USERNAME, or cargo cannot write
# to its own registry cache.
RUN chown -R $USERNAME:$USERNAME /app /usr/local/cargo

ENV TZ="America/Belize"

USER $USERNAME

CMD ["cargo", "watch", "-x", "run"]

# ──── Test ──────────────────────────────────
FROM base AS test

COPY . .

CMD ["cargo", "test", "--all-targets"]

# ──── Builder ──────────────────────────────────
FROM base AS builder

COPY . .

RUN touch src/main.rs && cargo build --release --locked

# ──── Prod ──────────────────────────────────
FROM debian:bookworm-slim AS production

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -g 1000 appuser \
    && useradd -m -u 1000 -g 1000 appuser

ENV TZ="America/Belize"

WORKDIR /app

COPY --from=builder /app/target/release/server /usr/local/bin/server

RUN mkdir -p /app/storage/logs && chown -R appuser:appuser /app

USER appuser

EXPOSE 3000 50051

CMD ["server"]
