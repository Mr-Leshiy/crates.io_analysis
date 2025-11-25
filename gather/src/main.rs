mod analyze;
mod crates_index;
mod crates_io;

use std::{
    fs::File,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    thread,
};

use bytes::Buf;
use clap::{Parser, ValueEnum};
use flate2::bufread::GzDecoder;
use futures::StreamExt;
use indicatif::ProgressStyle;
use rayon::ThreadPoolBuilder;
use tar::Archive;
use tempdir::TempDir;
use tracing::{Span, level_filters::LevelFilter};
use tracing_indicatif::{IndicatifLayer, span_ext::IndicatifSpanExt};
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    analyze::{analyze, types::AnalyzedCrateInfo},
    crates_index::get_all_crates_versions,
    crates_io::CratesIoApi,
};

#[derive(Parser, Debug)]
struct Cli {
    /// Path to the downloaded crates index.
    #[clap(long)]
    crates_index: PathBuf,

    /// Number of simultaneously processed crates.
    #[clap(long, default_value_t = 20)]
    simultaneous: usize,

    /// Output csv filename path
    #[clap(long, default_value = "crates_info.csv")]
    out: PathBuf,

    #[clap(long, default_value = "info")]
    log_level: LogLevel,
}

/// All valid logging levels
#[derive(ValueEnum, Clone, Copy, Debug)]
pub(crate) enum LogLevel {
    /// Debug messages
    Debug,
    /// Informational Messages
    Info,
    /// Warnings
    Warn,
    /// Errors
    Error,
}

impl From<LogLevel> for LevelFilter {
    fn from(val: LogLevel) -> Self {
        match val {
            LogLevel::Debug => Self::DEBUG,
            LogLevel::Info => Self::INFO,
            LogLevel::Warn => Self::WARN,
            LogLevel::Error => Self::ERROR,
        }
    }
}

fn setup_tracing(log_level: LogLevel) -> anyhow::Result<()> {
    let filter = EnvFilter::builder()
        .from_env()?
        .add_directive(LevelFilter::from(log_level).into())
        .add_directive("cargo_deny=off".parse()?)
        .add_directive("cargo=off".parse()?);

    let pb_style = ProgressStyle::with_template("{bar:60} ({pos}/{len}, ETA {eta})")?;
    let indicatif_layer = IndicatifLayer::new()
        .with_progress_style(pb_style)
        .with_filter(filter.clone());

    let log_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .without_time()
        .with_writer(indicatif_layer.inner().get_stderr_writer())
        .with_filter(filter);

    tracing_subscriber::registry()
        .with(log_layer)
        .with(indicatif_layer)
        .init();
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::try_parse()?;

    setup_tracing(cli.log_level)?;

    let num_threads = thread::available_parallelism().map_or(1, NonZeroUsize::get);
    tracing::info!(cli = ?cli, num_threads = num_threads, "Starting analyzing crates from crates.io...");

    ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build_global()?;

    let all_crates = get_all_crates_versions(&cli.crates_index, true)?;
    let all_crates = all_crates
        .into_iter()
        .map(|c| (c.name, c.version.to_string()))
        .collect::<Vec<_>>();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let temp = TempDir::new("crates_io")?;
        let api = CratesIoApi::new()?;

        let crates_info =
            process_all(all_crates.as_slice(), &api, temp.path(), cli.simultaneous).await?;
        write_to_csv(&cli.out, &crates_info)?;
        Ok(())
    })
}

#[tracing::instrument(skip_all)]
async fn process_all(
    crates: &[(String, String)],
    api: &CratesIoApi,
    out: &Path,
    simultaneous: usize,
) -> anyhow::Result<Vec<AnalyzedCrateInfo>> {
    let span: Span = Span::current();
    span.pb_set_length(crates.len().try_into()?);
    span.pb_set_finish_message("Processing all crates completed");

    let iter = crates.iter().map(|(name, version)| async move {
        let res = process(api, name.as_str(), version.as_str(), out).await?;
        // updating progress bar
        {
            let span = Span::current();
            span.pb_inc(1);
            tracing::info!(name = name, version = version, "Crate analyzed");
        }
        Ok(res)
    });

    let res: Vec<anyhow::Result<_>> = futures::stream::iter(iter)
        .buffer_unordered(simultaneous)
        .collect()
        .await;

    Ok(res
        .into_iter()
        .inspect(|v| {
            if let Err(err) = v {
                tracing::error!(error = err.to_string(), "Failing to process crate")
            }
        })
        .flatten()
        .collect())
}

async fn process(
    api: &CratesIoApi,
    crate_name: &str,
    crate_version: &str,
    out: &Path,
) -> anyhow::Result<AnalyzedCrateInfo> {
    let crate_bytes = api.download_crate(crate_name, crate_version).await?;
    let crate_dir = unpack_crate_to(crate_name, crate_version, crate_bytes, out)?;
    analyze(api, &crate_dir).await
}

fn unpack_crate_to(
    name: &str,
    version: &str,
    bytes: bytes::Bytes,
    out: &Path,
) -> anyhow::Result<PathBuf> {
    let gz: GzDecoder<bytes::buf::Reader<bytes::Bytes>> = GzDecoder::new(bytes.reader());
    let mut archive = Archive::new(gz);
    archive.unpack(out)?;
    Ok(out.join(format!("{name}-{version}")).to_path_buf())
}

#[tracing::instrument(skip_all)]
fn write_to_csv(out: &Path, crates: &[AnalyzedCrateInfo]) -> anyhow::Result<()> {
    let span = Span::current();
    span.pb_set_length(crates.len().try_into()?);
    span.pb_set_finish_message("Writing into csv completed");

    let mut csv_w = csv::WriterBuilder::new().from_writer(File::create(out)?);
    AnalyzedCrateInfo::write_header(&mut csv_w)?;

    for c in crates {
        c.write_record(&mut csv_w)?;
        span.pb_inc(1);
    }

    Ok(())
}
