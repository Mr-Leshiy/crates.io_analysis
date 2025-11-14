use std::{collections::BTreeMap, ops::AddAssign};

use cargo_deny::advisories;
use strum::IntoEnumIterator;

pub struct AnalyzedCrateInfo {
    pub name: String,
    pub version: String,
    pub downloads: u32,
    pub created_at: String,
    pub info: CrateInfo,
}

#[derive(Debug, Clone)]
pub struct CrateInfo {
    pub direct_deps: usize,
    pub all_deps: usize,
    pub advisories: AdvisoriesResults,
}

#[derive(Debug, Clone)]
pub struct AdvisoriesResults(BTreeMap<advisories::Code, u32>);

impl AnalyzedCrateInfo {
    pub fn write_header<W: std::io::Write>(csv_w: &mut csv::Writer<W>) -> anyhow::Result<()> {
        csv_w.write_record(
            [
                "name".to_string(),
                "version".to_string(),
                "downloads".to_string(),
                "created_at".to_string(),
                "direct_deps".to_string(),
                "all_deps".to_string(),
            ]
            .into_iter()
            .chain(
                advisories::Code::iter()
                    .map(|v| cargo_deny::diag::DiagnosticCode::Advisory(v).qualified_str()),
            ),
        )?;
        Ok(())
    }

    pub fn write_record<W: std::io::Write>(
        &self,
        csv_w: &mut csv::Writer<W>,
    ) -> anyhow::Result<()> {
        csv_w.write_field(&self.name)?;
        csv_w.write_field(&self.version)?;
        csv_w.write_field(self.downloads.to_string())?;
        csv_w.write_field(&self.created_at)?;
        csv_w.write_field(self.info.direct_deps.to_string())?;
        csv_w.write_field(self.info.all_deps.to_string())?;

        for fails in self.info.advisories.0.values() {
            csv_w.write_field(fails.to_string())?;
        }

        csv_w.write_record(None::<&[u8]>)?;
        Ok(())
    }
}

impl AdvisoriesResults {
    pub fn new() -> Self {
        let mut res = BTreeMap::<_, u32>::new();
        for c in advisories::Code::iter() {
            res.insert(c, 0);
        }
        Self(res)
    }

    pub fn inc(&mut self, code: advisories::Code) -> anyhow::Result<()> {
        self.0
            .get_mut(&code)
            .ok_or(anyhow::anyhow!(
                "advisory result entry must be already exists"
            ))?
            .add_assign(1);
        Ok(())
    }
}
