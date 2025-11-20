# Rust `crates.io` Analysis for Security and Reliability

This project is focused on analyzing Rust packages from [`crates.io`](https://crates.io) to evaluate them based on security, reliability, and other software quality metrics.
The analysis is driven by the [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) tool, which provides automated auditing capabilities for Rust dependencies.

## Build and run gathering tool

### Build

```shell
earthly +gather-build
```

### Run

```shell
docker run -ti --name gather gather:latest /bin/bash
./target/release/gather --crates-index /crates_index
```

### Copy

```shell
docker cp gather:app/crates_info.csv .
```

## Collected data

Inside the `data` directory, you'll find pre-collected data that you can analyze on your own !

## Analyze
```shell
uv run analyze.py --csv <file_1.csv> --csv <file_2.csv> ...
```
