mod crates_io;

use std::path::PathBuf;

use clap::Parser;

use crate::crates_io::CratesIoApi;

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
    loop {
        let resp = api.get_crates_names(cli.crates_num, next_page).await?;
        next_page = resp.1;
        for name in resp.0 {
            for crate_ver in api.get_crate_versions(&name).await? {
                let version = &crate_ver.version;
                if let Err(e) = api
                    .download_and_unpack_crate_to(&name, version, format!("{name}_{version}").into())
                    .await
                {
                    tracing::error!(
                        crate_name = name,
                        crate_version = version,
                        err = e.to_string(),
                        "Cannot download crate, skipping..."
                    );
                }
            }
        }
        if next_page.is_none() {
            break;
        }
        break;
    }

    Ok(())
}
