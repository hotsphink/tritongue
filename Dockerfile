# Build image.

FROM rust:1.68 AS builder
LABEL maintainer="Steve Fink <sphink@gmail.com>"

RUN mkdir -p /build/modules

COPY ./Cargo.* /build/
COPY ./src /build/src
COPY ./wit /build/wit
COPY ./modules /build/modules/

# Compile the host.
WORKDIR /build/src
RUN cargo build --release

# Set up protoc.
ENV PROTOC_ZIP=protoc-23.0-linux-x86_64.zip

RUN curl -OL https://github.com/protocolbuffers/protobuf/releases/download/v23.0/$PROTOC_ZIP && \
    unzip -o $PROTOC_ZIP -d /usr/local bin/protoc && \
    unzip -o $PROTOC_ZIP -d /usr/local 'include/*' && \
    chmod +x /usr/local/bin/protoc && \
    rm -f $PROTOC_ZIP

ENV PROTOC=/usr/local/bin/protoc

# Install the pinned version of cargo-component.
WORKDIR /build/modules
RUN ./install-cargo-component.sh && \
    rustup component add rustfmt && \
    rustup target add wasm32-unknown-unknown
RUN cargo component build --release --target=wasm32-unknown-unknown

# Actual image.
FROM debian:bullseye-slim

RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/* && \
    update-ca-certificates && \
    mkdir -p /opt/tritongue/data && \
    mkdir -p /opt/tritongue/modules/target/wasm32-unknown-unknown/release

COPY --from=builder /build/target/release/tritongue /opt/tritongue/tritongue
COPY --from=builder \
    /build/modules/target/wasm32-unknown-unknown/release/*.wasm \
    /opt/tritongue/modules/target/wasm32-unknown-unknown/release

ENV MATRIX_STORE_PATH /opt/tritongue/data/cache
ENV REDB_PATH /opt/tritongue/data/db

VOLUME /opt/tritongue/data

WORKDIR /opt/tritongue
CMD /opt/tritongue/tritongue
