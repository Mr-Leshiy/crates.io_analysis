mod analyze;
mod crates_index;
mod crates_io;

use std::{fs::File, num::NonZeroUsize, path::PathBuf, thread};

use clap::{Parser, ValueEnum};
use rayon::ThreadPoolBuilder;
use tracing::level_filters::LevelFilter;
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    analyze::types::AnalyzedCrateInfo,
    crates_index::{download_all_crates_versions, get_all_crates_versions},
};

#[derive(Parser, Debug)]
struct Cli {
    /// Path to the downloaded crates index.
    #[clap(long)]
    crates_index: PathBuf,

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
    let indicatif_layer = IndicatifLayer::new();

    let filter = EnvFilter::builder()
        .from_env()?
        .add_directive(LevelFilter::from(log_level).into())
        .add_directive("cargo_deny=error".parse()?)
        .add_directive("cargo=error".parse()?);

    let log_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .without_time()
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
    let mut csv_w = csv::WriterBuilder::new().from_writer(File::create(cli.out)?);
    AnalyzedCrateInfo::write_header(&mut csv_w)?;

    let all_crates = get_all_crates_versions(&cli.crates_index)?;
    download_all_crates_versions(&all_crates)?;

    Ok(())
}

// fn process_crates(crates: &[PathBuf]) -> anyhow::Result<()> {
//     let pb_style =
//         ProgressStyle::with_template("{bar:60} ({pos}/{len}, ETA {eta}) {wide_msg}").unwrap();

//     let span = Span::current();
//     span.pb_set_style(&pb_style);
//     span.pb_set_length(crates.len().try_into()?);

//     crates.par_iter().try_for_each(|p| -> anyhow::Result<()> {
//         let name = p
//             .file_name()
//             .map(OsStr::to_str)
//             .flatten()
//             .ok_or(anyhow::anyhow!("Must have a file name"))?;
//         span.pb_set_message(name);
//         span.pb_inc(1);
//         Ok(())
//     })?;

//     Ok(())
// }
// async fn process_crate_version(
//     api: &CratesIoApi,
//     crate_name: &CrateName,
//     crate_ver: &CrateVersion,
//     path: &Path,
// ) -> anyhow::Result<Option<analyzed_info::CrateInfo>> {
//     match api
//         .download_and_unpack_crate_to(crate_name, crate_ver, path)
//         .await
//     {
//         Ok(crate_dir) => analyze(&crate_dir).await,
//         Err(e) => {
//             tracing::error!(
//                 crate_name = crate_name,
//                 crate_version = crate_ver,
//                 err = e.to_string(),
//                 "Cannot download crate, skipping..."
//             );
//             Ok(None)
//         }
//     }
// }
