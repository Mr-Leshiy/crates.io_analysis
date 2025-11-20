use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CrateStatsResp {
    pub version: CrateStats,
}

#[derive(Debug, Deserialize)]
pub struct CrateStats {
    pub downloads: u32,
    pub created_at: String,
}
