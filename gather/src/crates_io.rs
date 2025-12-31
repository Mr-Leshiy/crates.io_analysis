use std::time::Duration;

use bytes::Bytes;
use reqwest::{Client, ClientBuilder};
use tokio::sync::Mutex;

use crate::types::{CrateStats, CrateStatsResp};

const CRATES_IO_URL: &str = "https://crates.io/api";
const CRATE_IO_STATIC_DOWNLOAD_URL: &str = "https://static.crates.io/crates";
const USER_AGENT: &str = "crates.io_analysis (https://github.com/Mr-Leshiy/crates.io_analysis)";

pub struct CratesIoApi(Mutex<CratesIoApiInner>);

struct CratesIoApiInner {
    c: Client,
}

impl CratesIoApi {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self(Mutex::new(CratesIoApiInner {
            c: ClientBuilder::new().user_agent(USER_AGENT).build()?,
        })))
    }

    pub async fn reset(&self) -> anyhow::Result<()> {
        let mut inner = self.0.lock().await;
        tokio::time::sleep(Duration::from_secs(30)).await;
        inner.c = ClientBuilder::new().user_agent(USER_AGENT).build()?;
        Ok(())
    }

    async fn get(&self, url: String) -> anyhow::Result<reqwest::Response> {
        let inner = self.0.lock().await;
        let resp = inner.c.get(url).send().await?;
        Ok(resp)
    }

    pub async fn get_crate_stats(&self, name: &str, version: &str) -> anyhow::Result<CrateStats> {
        for attempt in 1..6 {
            let resp = self
                .get(format!("{CRATES_IO_URL}/v1/crates/{name}/{version}"))
                .await?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await?;
                tracing::error!(
                    status_code = ?status,
                    body = body,
                    attempt = attempt,
                    crate_name = name,
                    "Failled to call 'crates.io/v1/crates/{{name}}/{{version}}' endpoint. Retrying..."
                );
                self.reset().await?;
                continue;
            }

            let resp = serde_json::from_slice::<CrateStatsResp>(&resp.bytes().await?)?;
            return Ok(resp.version);
        }
        anyhow::bail!("Failled to call 'crates.io/v1/crates/{name}/{version}' endpoint");
    }

    pub async fn download_crate(&self, name: &str, version: &str) -> anyhow::Result<Bytes> {
        for attempt in 1..6 {
            let resp = self
                .get(format!(
                    "{CRATE_IO_STATIC_DOWNLOAD_URL}/{name}/{name}-{version}.crate"
                ))
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await?;
                tracing::error!(
                    status_code = ?status,
                    body = body,
                    attempt = attempt,
                    crate_name = name,
                    version = version,
                    "Failled to download crate. Retrying..."
                );
                self.reset().await?;
                continue;
            }
            return Ok(resp.bytes().await?);
        }
        anyhow::bail!("Failled to download crate {name}-{version}");
    }
}
