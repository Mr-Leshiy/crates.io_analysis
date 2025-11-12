mod analyze;
mod analyzed_info;
mod crates_io;

use std::{
    fs::File,
    path::{Path, PathBuf},
};

use clap::Parser;

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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let cli = Cli::try_parse()?;
    tracing::info!(cli = ?cli, "Starting downloading and analyzing crates from crates.io...");

    let api = CratesIoApi::new()?;
    let mut next_page = cli.next_page;
    let mut csv_w = csv::WriterBuilder::new().from_writer(File::create(cli.out)?);
    AnalyzedCrateInfo::write_header(&mut csv_w)?;
    loop {
        let resp = api.get_crates_names(cli.crates_num, next_page).await?;
        next_page = resp.1;

        // for name in ["did-resolver-cheqd".to_string()] {
        for name in resp.0 {
            let tmp_dir = tempdir::TempDir::new("crates_io")?;

            for crate_ver in api.get_crate_versions(&name).await? {
                if let Some(info) = process_crate_version(
                    &api,
                    &name,
                    &crate_ver.version,
                    tmp_dir.path(), // &Path::new("/Users/alexeypoghilenkov/Projects/crates.io_analyses/gather"),
                )
                .await?
                {
                    analyzed_info::AnalyzedCrateInfo {
                        name: name.clone(),
                        version: crate_ver.version,
                        downloads: crate_ver.downloads,
                        created_at: crate_ver.created_at,
                        info,
                    }
                    .write_record(&mut csv_w)?;
                }
                break;
            }
        }
        if next_page.is_none() {
            break;
        }
        break;
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
        .download_and_unpack_crate_to(&crate_name, crate_ver, path)
        .await
    {
        Ok(crate_dir) => analyze(&crate_dir).await.map(Some),
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
