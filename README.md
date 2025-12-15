# Rust `crates.io` Analysis for Security and Reliability

This project is focused on analyzing Rust packages from [`crates.io`](https://crates.io) to evaluate them based on security, reliability, and other software quality metrics.
The analysis is driven by the [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) tool, which provides automated auditing capabilities for Rust dependencies.

## Build and run gathering tool

### Build

Need to fetch the latest `crates.io` index state
```shell
mkdir crates_index
git clone https://github.com/rust-lang/crates.io-index /crates_index
```

Build `gather` cli tool
```shell
cargo b --release
```

### Run

```shell
./target/release/gather --crates-index /crates_index
```

## Collected data

Inside the `data` directory, you'll find pre-collected data that you can analyze on your own !

## Analyze
```shell
uv run analyze.py --csv <file_1.csv> --csv <file_2.csv> ...
```
