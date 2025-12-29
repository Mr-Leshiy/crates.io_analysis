mod deny;

use std::{collections::HashMap, path::Path};

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

use crate::types::{AdvisoriesResults, DepsInfo};

pub fn analyze(crate_dir: &Path) -> anyhow::Result<(AdvisoriesResults, DepsInfo)> {
    let disable_stderr = gag::Gag::stderr();
    let disable_stdout = gag::Gag::stdout();

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

    let deps_info = get_deps_info(&ws)?;
    let advisories = deny::cargo_deny_advisories_check(&ws)?;

    tracing::debug!(crate_dir = ?crate_dir, deps_info = ?deps_info, advisories = ?advisories, "Crate analyzed.");

    std::mem::drop(disable_stderr);
    std::mem::drop(disable_stdout);

    Ok((advisories, deps_info))
}

fn get_deps_info(ws: &Workspace<'_>) -> anyhow::Result<DepsInfo> {
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

    let deps = DepsInfo {
        direct_deps,
        all_deps,
    };

    Ok(deps)
}
