mod analyze;
mod analyzed_info;
mod crates_io;

use std::{
    fs::File,
    path::{Path, PathBuf},
};

use clap::{Parser, ValueEnum};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

use crate::{
    analyze::analyze,
    analyzed_info::AnalyzedCrateInfo,
    crates_io::{CrateName, CrateVersion, CratesIoApi},
};

#[derive(Parser, Debug)]
struct Cli {
    /// Number of crates for simultaneous download and processing
    #[clap(long, default_value_t = 1)]
    crates_num: u8,

    /// crates.io 'v1/crates' 'seek' query argument
    #[clap(long)]
    next_page: Option<String>,

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::try_parse()?;

    let filter = EnvFilter::builder()
        .from_env()?
        .add_directive(LevelFilter::from(cli.log_level).into())
        .add_directive("cargo_deny=error".parse()?)
        .add_directive("cargo=error".parse()?);

    tracing_subscriber::fmt().with_env_filter(filter).init();

    tracing::info!(cli = ?cli, "Starting downloading and analyzing crates from crates.io...");

    let api = CratesIoApi::new()?;
    let mut next_page = cli.next_page;
    let mut csv_w = csv::WriterBuilder::new().from_writer(File::create(cli.out)?);
    AnalyzedCrateInfo::write_header(&mut csv_w)?;
    let mut analyzed = 0;
    loop {
        let resp = api.get_crates_names(cli.crates_num, next_page).await?;
        next_page = resp.1;

        for name in resp.0 {
            let tmp_dir = tempdir::TempDir::new("crates_io")?;

            for crate_ver in api.get_crate_versions(&name).await? {
                if let Some(info) =
                    process_crate_version(&api, &name, &crate_ver.version, tmp_dir.path()).await?
                {
                    AnalyzedCrateInfo {
                        name: name.clone(),
                        version: crate_ver.version,
                        downloads: crate_ver.downloads,
                        created_at: crate_ver.created_at,
                        info,
                    }
                    .write_record(&mut csv_w)?;
                    analyzed += 1;
                }
            }
        }
        tracing::info!(analyzed = analyzed, next_page = next_page);
        if next_page.is_none() {
            break;
        }
    }

    Ok(())
}

async fn process_crate_version(
    api: &CratesIoApi,
    crate_name: &CrateName,
    crate_ver: &CrateVersion,
    path: &Path,
) -> anyhow::Result<Option<analyzed_info::CrateInfo>> {
    match api
        .download_and_unpack_crate_to(crate_name, crate_ver, path)
        .await
    {
        Ok(crate_dir) => analyze(&crate_dir).await,
        Err(e) => {
            tracing::error!(
                crate_name = crate_name,
                crate_version = crate_ver,
                err = e.to_string(),
                "Cannot download crate, skipping..."
            );
            Ok(None)
        }
    }
}
