mod types;

use std::{
    ops::Not,
    path::{Path, PathBuf},
    time::Duration,
};

use bytes::Buf;
use flate2::read::GzDecoder;
use indicatif::ProgressStyle;
use reqwest::{Client, ClientBuilder};
use tar::Archive;
use tokio::sync::Mutex;

use tracing::Span;
use tracing_indicatif::span_ext::IndicatifSpanExt;
pub use types::{CrateName, CrateVersionInfo, NextPage};

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

    pub async fn get_crates_names(
        &self,
        per_page: u8,
        next_page: NextPage,
    ) -> anyhow::Result<(Vec<CrateName>, NextPage)> {
        let args = if let Some(next_page) = next_page {
            format!("?sort=new&include_yanked=no&per_page={per_page}&{next_page}")
        } else {
            format!("?sort=new&include_yanked=no&per_page={per_page}")
        };

        for attempt in 1..11 {
            let resp = {
                let c = self.c.lock().await;
                c.get(format!("{CRATES_IO_URL}/v1/crates{args}"))
                    .header(USER_AGENT, USER_AGENT)
                    .send()
                    .await?
            };

            if !resp.status().is_success() {
                tracing::error!(status_code = ?resp.status(), attempt=attempt,  "Failled to call 'crates.io/v1/crates' endpoint. Retrying...");
                self.reset().await?;
                continue;
            }

            let resp = serde_json::from_slice::<types::CratesResp>(&resp.bytes().await?)?;
            let next_page = resp.meta.next_page.and_then(|v| {
                v.split("&").find_map(|pair| {
                    pair.find("seek=")?;
                    Some(pair.to_string())
                })
            });
            tracing::debug!(
                next_page = next_page,
                total_crates_amount = resp.meta.total,
                "Successfully get crate names info"
            );
            return Ok((
                resp.crates
                    .into_iter()
                    .filter_map(|v| v.yanked.not().then_some(v.name))
                    .collect(),
                next_page,
            ));
        }
        anyhow::bail!("Failled to call 'crates.io/v1/crates' endpoint");
    }

    pub async fn get_crate_versions(
        &self,
        name: &CrateName,
    ) -> anyhow::Result<Vec<CrateVersionInfo>> {
        for attempt in 1..11 {
            let resp = {
                let c = self.c.lock().await;
                c.get(format!("{CRATES_IO_URL}/v1/crates/{name}/versions"))
                    .header(USER_AGENT, USER_AGENT)
                    .send()
                    .await?
            };
            if !resp.status().is_success() {
                tracing::error!(status_code = ?resp.status(), attempt=attempt, crate_name = name,  "Failled to call 'crates.io/v1/crates/{{name}}/versions' endpoint. Retrying...");
                self.reset().await?;
                continue;
            }

            let resp = serde_json::from_slice::<types::CrateVersionsResp>(&resp.bytes().await?)?;
            tracing::debug!(
                versions_count = resp.versions.len(),
                crate_name = name,
                "Successfully get crate versions info."
            );
            return Ok(resp
                .versions
                .into_iter()
                .filter_map(|v| v.yanked.not().then_some(v))
                .collect());
        }
        anyhow::bail!("Failled to call 'crates.io/v1/crates/{{name}}/versions' endpoint");
    }

    #[tracing::instrument(skip_all)]
    pub async fn download_and_unpack_crates_to(
        &self,
        crates: &[(String, String)],
        out: &Path,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let pb_style =
            ProgressStyle::with_template("{bar:60} ({pos}/{len}, ETA {eta}) {wide_msg}")?;

        let span = Span::current();
        span.pb_set_style(&pb_style);
        span.pb_set_length(crates.len().try_into()?);
        span.pb_set_finish_message(&format!("Downloading all crates versions from completed"));

        futures::future::join_all(crates.iter().map(|(name, version)| {
            // updating progress bar
            {
                span.pb_set_message(&format!("{name}-{version}"));
                span.pb_inc(1);
            }

            self.download_and_unpack_crate_to(name.as_str(), version.as_str(), out)
        }))
        .await
        .into_iter()
        .collect::<anyhow::Result<Vec<_>>>()
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
