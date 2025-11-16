mod deny;
pub mod types;

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

use crate::analyze::types::CrateInfo;

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

pub async fn analyze(crate_dir: &Path) -> anyhow::Result<Option<CrateInfo>> {
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
    let Some(advisories) = deny::cargo_deny_advisories_check(&ws)? else {
        return Ok(None);
    };

    let info = CrateInfo {
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
