mod deny;
pub mod types;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

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
use indicatif::ProgressStyle;
use tracing::Span;
use tracing_indicatif::span_ext::IndicatifSpanExt;

use crate::{
    analyze::types::{AdvisoriesResults, AnalyzedCrateInfo, CrateMetaInfo},
    crates_io::CratesIoApi,
};

#[tracing::instrument(skip_all)]
pub async fn analyze_and_record_crates<W: std::io::Write>(
    api: &CratesIoApi,
    crates_dirs: &[PathBuf],
    csv_w: &mut csv::Writer<W>,
) -> anyhow::Result<()> {
    let pb_style = ProgressStyle::with_template("{bar:60} ({pos}/{len}, ETA {eta}) {wide_msg}")?;

    let span: Span = Span::current();
    span.pb_set_style(&pb_style);
    span.pb_set_length(crates_dirs.len().try_into()?);
    span.pb_set_finish_message(&format!("Analyzing all crates completed"));

    for crate_dir in crates_dirs {
        let span = Span::current();

        if let Some(res) = analyze(api, crate_dir).await? {
            res.write_record(csv_w)?;
            span.pb_set_message(&format!("{}-{}", res.meta.name, res.meta.version));
        }

        span.pb_inc(1);
    }

    Ok(())
}

async fn analyze(api: &CratesIoApi, crate_dir: &Path) -> anyhow::Result<Option<AnalyzedCrateInfo>> {
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

    let meta = get_crate_meta(api, &ws).await?;

    // let Some(advisories) = deny::cargo_deny_advisories_check(&ws)? else {
    //     return Ok(None);
    // };

    let info = AnalyzedCrateInfo {
        meta,
        advisories: AdvisoriesResults::new(),
    };
    tracing::debug!(crate_dir = ?crate_dir, info = ?info, "Crate analyzed.");

    Ok(Some(info))
}

async fn get_crate_meta(api: &CratesIoApi, ws: &Workspace<'_>) -> anyhow::Result<CrateMetaInfo> {
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

    let name = member.name().to_string();
    let version = member.version().to_string();

    let stats = api.get_crate_stats(&name, &version).await?;

    Ok(CrateMetaInfo {
        name,
        version,
        downloads: stats.downloads,
        created_at: stats.created_at,
        direct_deps,
        all_deps,
    })
}
