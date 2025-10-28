mod crates_io;

use std::path::PathBuf;

use clap::Parser;

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

    let mut next_page = cli.next_page;
    loop {
        let resp = crates_io::get_crates_names(cli.crates_num, next_page).await?;
        next_page = resp.1;
        if next_page.is_none() {
            break;
        }
        let crates_names = resp.0;
        break;
    }

    Ok(())
}
