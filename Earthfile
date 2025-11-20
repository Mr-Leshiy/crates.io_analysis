VERSION 0.8

gather-build:
    FROM rust:1.91-trixie

    RUN apt-get update && \
        apt-get install -y git pkg-config libssl-dev curl build-essential && \
        apt-get clean

    RUN mkdir crates_index
    RUN git clone https://github.com/rust-lang/crates.io-index /crates_index

    WORKDIR /app
    COPY ./gather .
    RUN cargo b --release
    SAVE IMAGE gather:latest