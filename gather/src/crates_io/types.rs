use serde::Deserialize;

pub type CrateName = String;
pub type CrateVersion = String;
pub type NextPage = Option<String>;

#[derive(Debug, Deserialize)]
pub struct CratesResp {
    pub crates: Vec<CrateNameInfo>,
    pub meta: Meta,
}

#[derive(Debug, Deserialize)]
pub struct CrateNameInfo {
    pub name: CrateName,
    pub yanked: bool,
}

#[derive(Debug, Deserialize)]
pub struct Meta {
    pub next_page: NextPage,
    pub total: u32,
}

#[derive(Debug, Deserialize)]
pub struct CrateVersionsResp {
    pub versions: Vec<CrateVersionInfo>,
}

#[derive(Debug, Deserialize)]
pub struct CrateVersionInfo {
    pub downloads: u32,
    pub yanked: bool,
    pub created_at: String,
    #[serde(rename = "num")]
    pub version: CrateVersion,
}
