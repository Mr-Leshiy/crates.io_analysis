use std::time::Duration;

use reqwest::{ClientBuilder, header::USER_AGENT};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct CratesResp {
    crates: Vec<CrateInfo>,
    meta: Meta,
}

#[derive(Debug, Deserialize)]
struct CrateInfo {
    name: String,
    yanked: bool,
}

#[derive(Debug, Deserialize)]
struct Meta {
    next_page: String,
    total: u32,
}

const CRATES_IO_URL: &str = "https://crates.io/api";
const USER_AGENT_HEADER: &str =
    "crates.io_analysis (https://github.com/Mr-Leshiy/crates.io_analysis)";

pub type CrateName = String;
pub type NextPage = Option<String>;

pub async fn get_crates_names(
    per_page: u8,
    next_page: NextPage,
) -> anyhow::Result<(Vec<CrateName>, NextPage)> {
    let args = if let Some(next_page) = next_page {
        format!("?sort=new&include_yanked=no&per_page={per_page}&{next_page}")
    } else {
        format!("?sort=new&include_yanked=no&per_page={per_page}")
    };

    for attempt in 1..6 {
        let c = ClientBuilder::new().build()?;
        let resp = c
            .get(format!("{CRATES_IO_URL}/v1/crates{args}"))
            .header(USER_AGENT, USER_AGENT_HEADER)
            .send()
            .await?;
        if !resp.status().is_success() {
            tracing::error!(status_code = ?resp.status(), attempt=attempt,  "Failled to call 'crates.io/v1/crates' endpoint. Retrying...");
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }

        let resp: CratesResp = serde_json::from_slice(&resp.bytes().await?)?;
        let next_page = resp.meta.next_page.split("&").find_map(|pair| {
            pair.find("seek=")?;
            Some(pair.to_string())
        });
        tracing::info!(next_page = next_page, total_crates_amount = resp.meta.total, crates = ?resp.crates);
        return Ok((
            resp.crates
                .into_iter()
                .filter_map(|v| v.yanked.then_some(v.name))
                .collect(),
            next_page,
        ));
    }
    anyhow::bail!("Failled to call 'crates.io/v1/crates' endpoint");
}
