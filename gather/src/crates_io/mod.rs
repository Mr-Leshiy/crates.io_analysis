mod types;

use std::{ops::Not, path::PathBuf, time::Duration};

use bytes::Buf;
use flate2::read::GzDecoder;
use reqwest::{
    Client, ClientBuilder,
    header::{CONTENT_TYPE, USER_AGENT},
};
use tar::Archive;
use tokio::sync::Mutex;

use crate::crates_io::types::{CrateName, CrateVersion, CrateVersionInfo, NextPage};

const CRATES_IO_URL: &str = "https://crates.io/api";
const USER_AGENT_HEADER: &str =
    "crates.io_analysis (https://github.com/Mr-Leshiy/crates.io_analysis)";

pub struct CratesIoApi {
    c: Mutex<Client>,
}

impl CratesIoApi {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            c: Mutex::new(ClientBuilder::new().build()?),
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
                    .header(USER_AGENT, USER_AGENT_HEADER)
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
            tracing::info!(
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
                    .header(USER_AGENT, USER_AGENT_HEADER)
                    .send()
                    .await?
            };
            if !resp.status().is_success() {
                tracing::error!(status_code = ?resp.status(), attempt=attempt, crate_name = name,  "Failled to call 'crates.io/v1/crates/{{name}}/versions' endpoint. Retrying...");
                self.reset().await?;
                continue;
            }

            let resp = serde_json::from_slice::<types::CrateVersionsResp>(&resp.bytes().await?)?;
            tracing::info!(
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

    pub async fn download_and_unpack_crate_to(
        &self,
        name: &CrateName,
        version: &CrateVersion,
        out: PathBuf,
    ) -> anyhow::Result<()> {
        tracing::info!(
            crate_name = name,
            crate_version = version,
            "Downloading crate..."
        );
        let resp = {
            let c = self.c.lock().await;
            c.get(format!(
                "{CRATES_IO_URL}/v1/crates/{name}/{version}/download"
            ))
            .header(USER_AGENT, USER_AGENT_HEADER)
            .send()
            .await?
        };

        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .ok_or(anyhow::anyhow!("Content-type is missing."))?;
        anyhow::ensure!(
            content_type.to_str()? == "application/gzip",
            "Content-type is {content_type:?}",
        );

        tracing::info!(
            crate_name = name,
            crate_version = version,
            "Unpacking crate..."
        );

        let bytes = resp.bytes().await?;
        let  gz: GzDecoder<bytes::buf::Reader<bytes::Bytes>> = GzDecoder::new(bytes.reader());
        let mut archive = Archive::new(gz);
        if let Err(err) = archive.unpack(out) {
            tracing::error!(err = ?err);
            return Err(err.into());
        }
        Ok(())
    }
}
