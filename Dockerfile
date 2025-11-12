FROM rust:1.91-slim-bookworm

RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev curl build-essential && \
    apt-get clean

WORKDIR /app
COPY ./gather .
RUN cargo b --release
ENTRYPOINT ["./target/release/gather"]