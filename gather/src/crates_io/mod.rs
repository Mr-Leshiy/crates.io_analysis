mod types;

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use bytes::Buf;
use flate2::read::GzDecoder;
use futures::{
    FutureExt, TryStreamExt,
    stream::{Collect, FuturesOrdered, FuturesUnordered, StreamExt},
};
use indicatif::ProgressStyle;
use reqwest::{Client, ClientBuilder};
use tar::Archive;
use tokio::sync::Mutex;

use tracing::Span;
use tracing_indicatif::span_ext::IndicatifSpanExt;
pub use types::CrateStats;

const CRATES_IO_URL: &str = "https://crates.io/api";
const CRATE_IO_STATIC_DOWNLOAD_URL: &str = "https://static.crates.io/crates";
const USER_AGENT: &str = "crates.io_analysis (https://github.com/Mr-Leshiy/crates.io_analysis)";

pub struct CratesIoApi {
    c: Mutex<Client>,
}

impl CratesIoApi {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            c: Mutex::new(ClientBuilder::new().user_agent(USER_AGENT).build()?),
        })
    }

    pub async fn reset(&self) -> anyhow::Result<()> {
        let mut c = self.c.lock().await;
        tokio::time::sleep(Duration::from_secs(5)).await;
        *c = ClientBuilder::new().build()?;
        Ok(())
    }

    pub async fn get_crate_stats(&self, name: &str, version: &str) -> anyhow::Result<CrateStats> {
        for attempt in 1..11 {
            let resp = {
                let c = self.c.lock().await;
                c.get(format!("{CRATES_IO_URL}/v1/crates/{name}/{version}"))
                    .send()
                    .await?
            };
            if !resp.status().is_success() {
                tracing::error!(status_code = ?resp.status(), attempt=attempt, crate_name = name,  "Failled to call 'crates.io/v1/crates/{{name}}/{{version}}' endpoint. Retrying...");
                self.reset().await?;
                continue;
            }

            let resp = serde_json::from_slice::<types::CrateStatsResp>(&resp.bytes().await?)?;
            return Ok(resp.version);
        }
        anyhow::bail!("Failled to call 'crates.io/v1/crates/{{name}}/{{version}}' endpoint");
    }

    #[tracing::instrument(skip_all)]
    pub async fn download_and_unpack_crates_to(
        &self,
        crates: &[(String, String)],
        out: &Path,
        num_threads: usize,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let pb_style =
            ProgressStyle::with_template("{bar:60} ({pos}/{len}, ETA {eta}) {wide_msg}")?;

        let span = Span::current();
        span.pb_set_style(&pb_style);
        span.pb_set_length(crates.len().try_into()?);
        span.pb_set_finish_message(&format!("Downloading and unpacking all crates completed"));

        let iter = crates.iter().map(|(name, version)| async move {
            let res = self
                .download_and_unpack_crate_to(name.as_str(), version.as_str(), out)
                .await?;
            // updating progress bar
            {
                let span = Span::current();
                span.pb_set_message(&format!("{name}-{version}"));
                span.pb_inc(1);
            }
            Ok(res)
        });
        let res: Vec<anyhow::Result<PathBuf>> = futures::stream::iter(iter)
            .buffer_unordered(num_threads)
            .collect()
            .await;
        Ok(res.into_iter().collect::<anyhow::Result<_>>()?)
    }

    async fn download_and_unpack_crate_to(
        &self,
        name: &str,
        version: &str,
        out: &Path,
    ) -> anyhow::Result<PathBuf> {
        tracing::debug!(
            crate_name = name,
            crate_version = version,
            "Downloading crate..."
        );
        let resp = {
            let c = self.c.lock().await;
            c.get(format!(
                "{CRATE_IO_STATIC_DOWNLOAD_URL}/{name}/{name}-{version}.crate"
            ))
            .send()
            .await?
        };

        anyhow::ensure!(
            resp.status().is_success(),
            "Failed to download {name}-{version}, {resp:?}",
        );

        tracing::debug!(
            crate_name = name,
            crate_version = version,
            "Unpacking crate..."
        );

        let bytes = resp.bytes().await?;
        let gz: GzDecoder<bytes::buf::Reader<bytes::Bytes>> = GzDecoder::new(bytes.reader());
        let mut archive = Archive::new(gz);

        archive.unpack(out)?;

        Ok(out.join(format!("{name}-{version}")).to_path_buf())
    }
}

#[tokio::test]
async fn download_and_unpack_crate_to_test() {
    use tempdir::TempDir;

    let api = CratesIoApi::new().unwrap();
    let temp = TempDir::new("download_and_unpack_crate_to_test").unwrap();

    api.download_and_unpack_crate_to("serde", "1.0.228", temp.path())
        .await
        .unwrap();
}
