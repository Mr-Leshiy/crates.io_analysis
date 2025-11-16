use std::{
    ffi::OsStr,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use indicatif::ProgressStyle;
use rayon::iter::{ParallelBridge, ParallelIterator};
use semver::Version;
use serde::Deserialize;
use tempdir::TempDir;
use tracing::{Level, Span, event};
use tracing_indicatif::span_ext::IndicatifSpanExt;
use walkdir::WalkDir;

#[derive(Deserialize, Debug)]
pub struct CrateVersion {
    pub name: String,
    #[serde(rename = "vers")]
    pub version: Version,
    pub yanked: bool,
}

#[tracing::instrument]
pub fn get_all_crates_versions(crates_index: &Path) -> anyhow::Result<Vec<CrateVersion>> {
    fn is_hidden(e: &walkdir::DirEntry) -> bool {
        e.file_name().to_str().is_some_and(|s| s.starts_with('.'))
    }

    fn is_file(e: &walkdir::DirEntry) -> bool {
        e.file_type().is_file()
    }

    fn is_config_file(e: &walkdir::DirEntry) -> bool {
        e.path().file_name() == Some(&OsStr::new("config.json"))
    }

    fn is_readme(e: &walkdir::DirEntry) -> bool {
        e.path().file_name() == Some(&OsStr::new("README.md"))
    }

    fn skip_entry(e: &walkdir::DirEntry) -> bool {
        !is_hidden(e) && is_file(e) && !is_config_file(e) && !is_readme(e)
    }

    let crates_number = git2::Repository::open(crates_index)?.index()?.len();
    let pb_style = ProgressStyle::with_template("{bar:60} ({pos}/{len}, ETA {eta}) {wide_msg}")?;

    let span = Span::current();
    span.pb_set_style(&pb_style);
    span.pb_set_length(crates_number.try_into()?);
    span.pb_set_finish_message(&format!(
        "Reading all crates versions from {crates_index:?} completed"
    ));

    let mut iter = WalkDir::new(crates_index).sort_by_file_name().into_iter();
    let root = iter
        .next()
        .ok_or(anyhow::anyhow!("Must have a root entry"))??;
    anyhow::ensure!(root.path().file_name() == crates_index.file_name());

    let git = iter
        .next()
        .ok_or(anyhow::anyhow!("Must have an '.git' entry"))??;
    anyhow::ensure!(git.path().file_name() == Some(OsStr::new(".git")));
    iter.skip_current_dir();

    let github = iter
        .next()
        .ok_or(anyhow::anyhow!("Must have an '.github' entry"))??;
    anyhow::ensure!(github.path().file_name() == Some(OsStr::new(".github")));
    iter.skip_current_dir();

    Ok(iter
        .par_bridge()
        .filter_map(|e| {
            e.inspect_err(|err| tracing::warn!(?err, "walkdir result is error"))
                .ok()
        })
        .filter(skip_entry)
        .filter_map(|d| {
            // updating progress bar
            {
                let file_name = d.path().file_name().and_then(|v| v.to_str());
                file_name.inspect(|v| span.pb_set_message(v));
                span.pb_inc(1);
            }

            read_crate_index_file(d.into_path())
                .inspect_err(|err| {
                    tracing::warn!(err = err.to_string(), "Cannot read crate vesions from file")
                })
                .ok()
        })
        .flatten()
        .filter(|v| !v.yanked)
        .collect())
}

fn read_crate_index_file(
    path: PathBuf,
) -> anyhow::Result<impl ParallelIterator<Item = CrateVersion>> {
    let f = File::open(&path)?;
    let deser = serde_json::Deserializer::from_reader(f);
    Ok(deser.into_iter::<CrateVersion>().par_bridge().filter_map({
        let path = path.clone();
        move |line| {
            line.inspect_err(|err| {
                tracing::warn!(err = err.to_string(), path=?path, "Cannot deserialize crate vesion")
            })
            .ok()
        }
    }))
}

/// Downloads all provided crates versions.
/// By the provided crates versions list, sets a dummy `Cargo.toml` project file,
/// listing all provided crates as dependencies.
/// The next step is to invoke `cargo fetch` on the freshly set crate.
/// By doing this the proper downloading, invoked by the 'cargo' itself.
pub fn download_all_crates_versions(crates_versions: &[CrateVersion]) -> anyhow::Result<()> {
    let tempdir = TempDir::new("dummy_project")?;

    let root_path = tempdir.path();
    prepare_dummy_rust_project(root_path, crates_versions)?;
    cargo_fetch(root_path)?;

    Ok(())
}

fn prepare_dummy_rust_project(
    root_path: &Path,
    crates_versions: &[CrateVersion],
) -> anyhow::Result<()> {
    // 2. Build the Cargo.toml content using the `toml` crate

    // Root map for the TOML structure
    let mut root = toml::map::Map::new();

    // [package] table
    let mut package_table = toml::map::Map::new();
    package_table.insert(
        "name".to_string(),
        toml::Value::String("dummy-project".to_string()),
    );
    package_table.insert(
        "version".to_string(),
        toml::Value::String("0.1.0".to_string()),
    );
    package_table.insert(
        "edition".to_string(),
        toml::Value::String("2024".to_string()),
    );
    root.insert(
        "package".to_string(),
        toml::Value::Table(package_table.into()),
    );

    // [dependencies] table
    let mut dependencies_table = toml::map::Map::new();
    for entry in crates_versions {
        dependencies_table.insert(
            entry.name.clone(),
            toml::Value::String(format!("={}", entry.version)),
        );
    }

    root.insert(
        "dependencies".to_string(),
        toml::Value::Table(dependencies_table),
    );

    // Serialize the structure to a formatted TOML string
    let cargo_toml_content = toml::to_string_pretty(&root)?;

    // Write Cargo.toml to the temporary directory
    let cargo_toml_path = root_path.join("Cargo.toml");
    let mut file = fs::File::create(cargo_toml_path)?;
    file.write_all(cargo_toml_content.as_bytes())?;

    // 4. Create dummy src/lib.rs (for a library project)
    let src_path = root_path.join("src");
    fs::create_dir(&src_path)?;
    // Create an empty lib.rs file
    let lib_rs_path = src_path.join("lib.rs");
    File::create(lib_rs_path)?;

    Ok(())
}

fn cargo_fetch(root_path: &Path) -> anyhow::Result<()> {
    let mut command = Command::new("cargo");
    command.arg("fetch");
    command.arg("--manifest-path");
    command.arg(root_path.join("Cargo.toml"));

    let status = command.status()?;
    anyhow::ensure!(
        status.success(),
        "'cargo fetch' wasn't finished sucessfully"
    );
    Ok(())
}
