# Rust `crates.io` Analysis for Security and Reliability

This project is focused on analyzing Rust packages from [`crates.io`](https://crates.io) to evaluate them based on security, reliability, and other software quality metrics.
The analysis is driven by the [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) tool, which provides automated auditing capabilities for Rust dependencies.

## Build and run gathering tool
```shell
docker build -t crates_gather:latest .
docker run --detach --name crates_gather crates_gather:latest
docker cp crates_gather:app/crates_info.csv .
```

## Collected data

Inside the `data` directory, you'll find pre-collected data that you can analyze on your own !

## Analyze
```shell
uv run analyze.py --csv <file_1.csv> --csv <file_2.csv> ...
```
