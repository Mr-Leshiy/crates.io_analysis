mod analyze;
mod crates_index;
mod crates_io;
mod types;

use std::{
    fs::File, num::NonZeroUsize, path::{Path, PathBuf}, sync::Arc, thread
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
    analyze::analyze, crates_index::get_all_crates_versions, crates_io::CratesIoApi,
    types::AnalyzedCrateInfo,
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

        let crates_info = process_all(all_crates.as_slice(), temp.path(), cli.simultaneous).await?;
        tracing::info!(
            analyzed_number = crates_info.len(),
            skipped = (all_crates.len() - crates_info.len()),
            "Crates analyzed"
        );
        write_to_csv(&cli.out, &crates_info)?;
        Ok(())
    })
}

#[tracing::instrument(skip_all)]
async fn process_all(
    crates: &[(String, String)],
    out: &Path,
    simultaneous: usize,
) -> anyhow::Result<Vec<AnalyzedCrateInfo>> {
    let span: Span = Span::current();
    span.pb_set_length(crates.len().try_into()?);
    span.pb_set_finish_message("Processing all crates completed");

    let apis = (0..simultaneous)
        .map(|_| CratesIoApi::new().map(Arc::new))
        .collect::<anyhow::Result<Vec<_>>>()?;

    let mut result = Vec::with_capacity(crates.len());
    for chunk in crates.chunks(apis.len()) {
        let handles = chunk.iter().zip(apis.iter()).map(|((name, version), api)| {
            tokio::spawn({
                let name = name.clone();
                let version = version.clone();
                let api = api.clone();
                let out = out.to_path_buf();
                async move {
                    let res = process(&api, name, version, &out)
                        .await
                        .inspect_err(|err| {
                            tracing::error!(error = err.to_string(), "Failing to process crate")
                        })
                        .ok()?;
                    // updating progress bar
                    {
                        let span = Span::current();
                        span.pb_inc(1);
                        tracing::info!(name = res.name, version = res.version, "Crate analyzed");
                    }
                    Some(res)
                }
            })
        });

        let res: Vec<_> = futures::stream::iter(handles)
            .buffer_unordered(chunk.len())
            .collect()
            .await;

        result.extend(res.into_iter().flat_map(Result::ok).flatten());
    }

    Ok(result)
}

async fn process(
    api: &CratesIoApi,
    crate_name: String,
    crate_version: String,
    out: &Path,
) -> anyhow::Result<AnalyzedCrateInfo> {
    let crate_bytes = api.download_crate(crate_name.as_str(), crate_version.as_str()).await?;
    let stats = api.get_crate_stats(crate_name.as_str(), crate_version.as_str()).await?;
    let crate_dir = unpack_crate_to(crate_name.as_str(), crate_version.as_str(), crate_bytes, out)?;
    let (advisories, deps) = analyze(&crate_dir)?;

    Ok(AnalyzedCrateInfo {
        name: crate_name,
        version: crate_version,
        stats,
        deps,
        advisories,
    })
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
