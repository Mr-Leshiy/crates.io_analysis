mod types;

use std::time::Duration;

use bytes::Bytes;
use reqwest::{Client, ClientBuilder};
use tokio::sync::Mutex;

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
                tracing::debug!(status_code = ?resp.status(), attempt=attempt, crate_name = name,  "Failled to call 'crates.io/v1/crates/{{name}}/{{version}}' endpoint. Retrying...");
                self.reset().await?;
                continue;
            }

            let resp = serde_json::from_slice::<types::CrateStatsResp>(&resp.bytes().await?)?;
            return Ok(resp.version);
        }
        anyhow::bail!("Failled to call 'crates.io/v1/crates/{name}/{version}' endpoint");
    }

    pub async fn download_crate(&self, name: &str, version: &str) -> anyhow::Result<Bytes> {
        for attempt in 1..11 {
            let resp = {
                let c = self.c.lock().await;
                c.get(format!(
                    "{CRATE_IO_STATIC_DOWNLOAD_URL}/{name}/{name}-{version}.crate"
                ))
                .send()
                .await?
            };

            if !resp.status().is_success() {
                tracing::debug!(status_code = ?resp.status(), attempt=attempt, crate_name = name, version = version,  "Failled to download crate. Retrying...");
                self.reset().await?;
                continue;
            }
            return Ok(resp.bytes().await?);
        }
        anyhow::bail!("Failled to download crate {name}-{version}");
    }
}
