use std::{collections::HashMap, fs::File, path::Path, sync::LazyLock};

use cargo::{
    GlobalContext,
    core::{
        Package, PackageId, Workspace,
        compiler::{CompileKind, RustcTargetData},
        dependency::DepKind,
        resolver::{CliFeatures, ForceAllTargets, HasDevUnits},
    },
    ops::resolve_ws_with_opts,
};
use cargo_deny::{
    CheckCtx, Spanned, UnvalidatedConfig,
    advisories::{self, cfg::Config},
    diag::{DiagnosticCode, DiagnosticOverrides, ErrorSink, Files, KrateSpans, Severity},
    utf8path,
};
use krates::{NoneFilter, Utf8PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;

use crate::analyzed_info;

pub fn unpack(path: &Path) -> anyhow::Result<()> {
    tracing::debug!(
        path = ?path,
        "Unpacking crate..."
    );

    let crate_file = File::open(path)?;
    let gz = GzDecoder::new(crate_file);
    let mut archive = Archive::new(gz);

    let out = path.parent().ok_or(anyhow::anyhow!(
        "Provided crate archive must have a parent directory"
    ))?;
    archive.unpack(out)?;
    Ok(())
}

pub async fn analyze(crate_dir: &Path) -> anyhow::Result<Option<analyzed_info::CrateInfo>> {
    tracing::debug!(
        crate_dir = ?crate_dir,
        "Analyzing crate..."
    );

    anyhow::ensure!(
        crate_dir.is_absolute(),
        "{crate_dir:?} must be an absolute path"
    );

    let ctx = GlobalContext::default()?;
    let ws = Workspace::new(&crate_dir.join("Cargo.toml"), &ctx)?;

    let (direct_deps, all_deps) = get_crate_deps_count(&ws)?;
    let Some(advisories) = cargo_deny_advisories_check(&ws)? else {
        return Ok(None);
    };

    let info = analyzed_info::CrateInfo {
        direct_deps,
        all_deps,
        advisories,
    };
    tracing::debug!(crate_dir = ?crate_dir, info = ?info, "Crate analyzed.");

    Ok(Some(info))
}

fn get_crate_deps_count(ws: &Workspace) -> anyhow::Result<(usize, usize)> {
    let &[member] = ws.members().collect::<Vec<_>>().as_slice() else {
        anyhow::bail!("Analyzed workspace must have only one member");
    };

    let specs = [member.package_id().to_spec()];
    let requested_kinds = CompileKind::from_requested_targets(ws.gctx(), &[])?;
    let mut target_data = RustcTargetData::new(ws, &requested_kinds)?;
    let cli_features = CliFeatures::new_all(true);
    let dry_run = false;
    let has_dev_units = HasDevUnits::No;
    let force_all = ForceAllTargets::No;

    let ws_resolve = resolve_ws_with_opts(
        ws,
        &mut target_data,
        &requested_kinds,
        &cli_features,
        &specs,
        has_dev_units,
        force_all,
        dry_run,
    )?;

    let mut package_map: HashMap<PackageId, &Package> = ws_resolve
        .pkg_set
        .packages()
        .map(|pkg| (pkg.package_id(), pkg))
        .collect();

    let p = package_map
        .remove(&member.package_id())
        .ok_or(anyhow::anyhow!("It must contain an original crate entry"))?;
    let direct_deps = p
        .dependencies()
        .iter()
        .filter(|d| !matches!(d.kind(), DepKind::Development))
        .count();
    // the rest of the items are ALL dependencies of the crate
    let all_deps = package_map.len();

    Ok((direct_deps, all_deps))
}

const ADVISORY_DB_PATH: LazyLock<anyhow::Result<Utf8PathBuf>> =
    LazyLock::new(|| utf8path(home::cargo_home()?.join("advisory-dbs")));

static ADVISORY_DB: LazyLock<anyhow::Result<advisories::DbSet>> = LazyLock::new(|| {
    let db_path = ADVISORY_DB_PATH
        .as_ref()
        .map_err(|e| anyhow::anyhow!("advisory db path must be initialized: {e}"))?
        .clone();
    advisories::DbSet::load(db_path, vec![], advisories::Fetch::Allow)
});

fn cargo_deny_advisories_check(
    ws: &Workspace,
) -> anyhow::Result<Option<analyzed_info::AdvisoriesResults>> {
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
        // For such rare cases just skip analyzing such crates.
        // Was failed for `swc_atlaskit_tokens/v0.0.3` crate.
        let Ok(md) = cargo::ops::output_metadata(ws, &options) else {
            return Ok(None);
        };
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

    let mut res = analyzed_info::AdvisoriesResults::new();
    for d in rx.iter().flat_map(|p| p.into_iter()) {
        if let Some(DiagnosticCode::Advisory(code)) = d.code {
            res.inc(code)?;
        }
    }

    Ok(Some(res))
}
