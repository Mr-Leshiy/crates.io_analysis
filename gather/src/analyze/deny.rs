use cargo::core::{Workspace, resolver::CliFeatures};
use cargo_deny::{
    CheckCtx, Spanned, UnvalidatedConfig,
    advisories::{self, cfg::Config},
    diag::{DiagnosticCode, DiagnosticOverrides, ErrorSink, Files, KrateSpans, Severity},
    utf8path,
};
use krates::{NoneFilter, Utf8PathBuf};
use std::sync::LazyLock;

use crate::analyze::types::AdvisoriesResults;

const ADVISORY_DB_PATH: LazyLock<anyhow::Result<Utf8PathBuf>> =
    LazyLock::new(|| utf8path(home::cargo_home()?.join("advisory-dbs")));

static ADVISORY_DB: LazyLock<anyhow::Result<advisories::DbSet>> = LazyLock::new(|| {
    let db_path = ADVISORY_DB_PATH
        .as_ref()
        .map_err(|e| anyhow::anyhow!("advisory db path must be initialized: {e}"))?
        .clone();
    advisories::DbSet::load(db_path, vec![], advisories::Fetch::Allow)
});

pub fn cargo_deny_advisories_check(ws: &Workspace) -> anyhow::Result<AdvisoriesResults> {
    let mut files = Files::new();
    // write down default `deny.default.toml`
    let cfg_id = files.add(
        cargo_deny::PathBuf::from("deny.default.toml"),
        String::new(),
    );

    let cfg = {
        let mut cfg = Config::default();
        cfg.yanked = Spanned::new(cargo_deny::LintLevel::Deny);
        cfg.db_path = Some(Spanned::new(
            ADVISORY_DB_PATH
                .as_ref()
                .map_err(|e| anyhow::anyhow!("advisory db path must be initialized: {e}"))?
                .clone(),
        ));
        cfg.validate(cargo_deny::cfg::ValidationContext {
            cfg_id,
            files: &mut files,
            diagnostics: &mut vec![],
        })
    };

    let advisory_db_set = ADVISORY_DB
        .as_ref()
        .map_err(|e| anyhow::anyhow!("advisory db initialized: {e}"))?;

    let krates = {
        let options = cargo::ops::OutputMetadataOptions {
            cli_features: CliFeatures::new_all(true),
            no_deps: false,
            version: 1,
            filter_platforms: vec![],
        };
        // its possible because of some rare feature flags with some optional dependencies,
        // with enabling all feature flags all together clashes dependencies.
        let md = cargo::ops::output_metadata(ws, &options)?;
        let md = serde_json::from_value::<krates::cm::Metadata>(serde_json::to_value(md)?)?;

        let mut kb = krates::Builder::new();
        kb.ignore_kind(krates::DepKind::Dev, krates::Scope::All);
        kb.build_with_metadata(md, NoneFilter)?
    };
    let indices = Some(advisories::Indices::load(
        &krates,
        utf8path(home::cargo_home()?)?,
    ));
    let krate_spans =
        &KrateSpans::synthesize(&krates, krates.workspace_root().as_str(), &mut files);
    let ctx = CheckCtx {
        cfg,
        krates: &krates,
        krate_spans,
        colorize: false,
        serialize_extra: false,
        log_level: log::LevelFilter::Off,
        files: &files,
    };
    let audit_reporter = Some(|_| {});

    let (tx, rx) = crossbeam_channel::unbounded();
    let overrides = Some(
        DiagnosticOverrides {
            code_overrides: DiagnosticCode::iter()
                .map(|v| (v.as_str(), Severity::Error))
                .collect(),
            level_overrides: vec![],
        }
        .into(),
    );
    let advisories_sink = ErrorSink {
        overrides,
        channel: tx,
    };

    advisories::check(
        ctx,
        advisory_db_set,
        audit_reporter,
        indices,
        advisories_sink,
    );

    let mut res = AdvisoriesResults::new();
    for d in rx.iter().flat_map(|p| p.into_iter()) {
        if let Some(DiagnosticCode::Advisory(code)) = d.code {
            res.inc(code)?;
        }
    }

    Ok(res)
}
