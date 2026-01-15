mod deny;

use std::{collections::HashMap, path::Path, sync::LazyLock};

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

use crate::types::{AdvisoriesResults, CrateType, DepsInfo};

static CARGO_GLOBAL_CTX: LazyLock<GlobalContext> =
    LazyLock::new(|| GlobalContext::default().expect("cannot initialise cargo global context"));

pub fn analyze(crate_dir: &Path) -> anyhow::Result<Vec<(AdvisoriesResults, DepsInfo, CrateType)>> {
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

    let ws = Workspace::new(&crate_dir.join("Cargo.toml"), &CARGO_GLOBAL_CTX)?;

    let &[member] = ws.members().collect::<Vec<_>>().as_slice() else {
        anyhow::bail!("Analyzed workspace must have only one member");
    };

    let mut res = Vec::new();

    // Has a 'bin' target
    if member.targets().iter().any(|t| t.is_bin()) {
        let (advisories, deps) = analyze_inner(&ws)?;
        res.push((advisories, deps, CrateType::Bin));
    }

    // Has a 'lib' target
    if member.targets().iter().any(|t| t.is_lib()) {
        let (advisories, deps) = analyze_inner(&ws)?;
        // Remove existing Cargo.lock file to pull the most recent one, with updated dependency tree.
        // Cargo.lock could be absent.
        let _ = std::fs::remove_file(crate_dir.join("Cargo.lock"));
        res.push((advisories, deps, CrateType::Lib));
    }

    tracing::debug!(crate_dir = ?crate_dir, "Crate analyzed.");

    std::mem::drop(disable_stderr);
    std::mem::drop(disable_stdout);

    Ok(res)
}

fn analyze_inner(ws: &Workspace<'_>) -> anyhow::Result<(AdvisoriesResults, DepsInfo)> {
    let deps_info = get_deps_info(&ws)?;
    let advisories = deny::cargo_deny_advisories_check(&ws)?;
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
