FROM rust:1.91-slim-bookworm

RUN apt-get update && \
    apt-get install -y git pkg-config libssl-dev curl build-essential && \
    apt-get clean

RUN cargo install get-all-crates
RUN mkdir crates_index
RUN git clone https://github.com/rust-lang/crates.io-index ./crates_index
RUN get-all-crates --index ./crates_index --out ./crates


WORKDIR /app
COPY ./gather .
RUN cargo b --release
ENTRYPOINT ["./target/release/gather --help"]