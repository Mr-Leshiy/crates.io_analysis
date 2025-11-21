use std::{
    ffi::OsStr,
    fs::File,
    path::{Path, PathBuf},
};

use rayon::iter::{ParallelBridge, ParallelIterator};
use semver::Version;
use serde::Deserialize;
use tracing::Span;
use tracing_indicatif::span_ext::IndicatifSpanExt;
use walkdir::WalkDir;

#[derive(Deserialize, Debug)]
pub struct CrateVersion {
    pub name: String,
    #[serde(rename = "vers")]
    pub version: Version,
    pub yanked: bool,
}

#[tracing::instrument(skip_all)]
pub fn get_all_crates_versions(
    crates_index: &Path,
    only_latest: bool,
) -> anyhow::Result<Vec<CrateVersion>> {
    fn is_hidden(e: &walkdir::DirEntry) -> bool {
        e.file_name().to_str().is_some_and(|s| s.starts_with('.'))
    }

    fn is_file(e: &walkdir::DirEntry) -> bool {
        e.file_type().is_file()
    }

    fn is_config_file(e: &walkdir::DirEntry) -> bool {
        e.path().file_name() == Some(OsStr::new("config.json"))
    }

    fn is_readme(e: &walkdir::DirEntry) -> bool {
        e.path().file_name() == Some(OsStr::new("README.md"))
    }

    fn skip_entry(e: &walkdir::DirEntry) -> bool {
        !is_hidden(e) && is_file(e) && !is_config_file(e) && !is_readme(e)
    }

    let crates_number = git2::Repository::open(crates_index)?.index()?.len();

    let span = Span::current();
    span.pb_set_length(crates_number.try_into()?);
    span.pb_set_finish_message(&format!(
        "Reading all crates versions from {crates_index:?} completed"
    ));

    let mut iter = WalkDir::new(crates_index).max_depth(3).sort_by_file_name().into_iter();
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

    let iter = iter.filter_entry(skip_entry)
        .par_bridge()
        .filter_map(|e| {
            e.inspect_err(|err| tracing::warn!(?err, "walkdir result is error"))
                .ok()
        })
        .inspect(|d|
            // updating progress bar
            {
                let file_name = d.path().file_name().and_then(|v| v.to_str());
                file_name.inspect(|v| span.pb_set_message(v));
                span.pb_inc(1);
            });

    if only_latest {
        Ok(iter
            .filter_map(|d| {
                read_latest_version_crate_index_file(d.into_path())
                    .inspect_err(|err| {
                        tracing::warn!(err = err.to_string(), "Cannot read crate vesions from file")
                    })
                    .ok()
            })
            .filter(|v| !v.yanked)
            .collect())
    } else {
        Ok(iter
            .filter_map(|d| {
                read_all_versions_crate_index_file(d.into_path())
                    .inspect_err(|err| {
                        tracing::warn!(err = err.to_string(), "Cannot read crate vesions from file")
                    })
                    .ok()
            })
            .flatten()
            .filter(|v| !v.yanked)
            .collect())
    }
}

fn read_latest_version_crate_index_file(path: PathBuf) -> anyhow::Result<CrateVersion> {
    let f = File::open(&path)?;
    let deser = serde_json::Deserializer::from_reader(f);
    deser
        .into_iter::<CrateVersion>()
        .par_bridge()
        .filter_map({
            let path = path.clone();
            move |line| {
                line.inspect_err(|err| {
                tracing::warn!(err = err.to_string(), path=?path, "Cannot deserialize crate vesion")
            })
            .ok()
            }
        })
        .max_by_key(|v| v.version.clone())
        .ok_or(anyhow::anyhow!("Must have at least one latest crate"))
}

fn read_all_versions_crate_index_file(
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
