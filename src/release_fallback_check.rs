//! Warn when a snapshot pins a package that a host cannot install.
//!
//! `gtc install` downloads a package's release archive when the manifest names
//! one for the host target, and otherwise falls back to `cargo binstall` from
//! crates.io. `--source github-releases` deliberately pins builds crates.io
//! never received, so a missing crates.io version is not an error: every
//! target that has an archive never looks there.
//!
//! It is a problem for a target WITHOUT an archive (the dev lane skips
//! `x86_64-apple-darwin` to save Actions cost). That host falls back to
//! crates.io, and a `crate@version` that was never published there fails the
//! whole install with an error that names neither the manifest nor the cause.
//! This check reports those combinations at snapshot time, as warnings: the
//! manifest is still valid for every other target.

use anyhow::{Context, Result};

use crate::release_cmd::{GTC_TARGETS, ToolchainManifest};

const CRATES_IO_API_BASE: &str = "https://crates.io/api/v1/crates";
const USER_AGENT: &str = concat!(
    "greentic-dev/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/greenticai/greentic-dev)"
);

/// Answers whether `crate_name@version` is published on crates.io.
pub trait CratesIoVersionLookup {
    fn is_published(&self, crate_name: &str, version: &str) -> Result<bool>;
}

pub struct CratesIoApi {
    client: reqwest::blocking::Client,
}

impl CratesIoApi {
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .context("failed to build crates.io API client")?;
        Ok(Self { client })
    }
}

impl CratesIoVersionLookup for CratesIoApi {
    fn is_published(&self, crate_name: &str, version: &str) -> Result<bool> {
        let url = format!("{CRATES_IO_API_BASE}/{crate_name}/{version}");
        let response = self
            .client
            .get(&url)
            .send()
            .with_context(|| format!("failed to GET {url}"))?;
        match response.status().as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            other => anyhow::bail!("crates.io API GET {url} returned HTTP {other}"),
        }
    }
}

/// One sentence per package that some target would install from crates.io
/// although the pinned version is not published there. A lookup that fails is
/// reported as a note instead: "could not ask" is not "not published".
pub fn fallback_warnings(
    manifest: &ToolchainManifest,
    lookup: &dyn CratesIoVersionLookup,
) -> Vec<String> {
    let mut out = Vec::new();
    for package in &manifest.packages {
        let covered: Vec<&str> = package
            .artifacts
            .iter()
            .flatten()
            .map(|artifact| artifact.target.as_str())
            .collect();
        let uncovered: Vec<&str> = GTC_TARGETS
            .iter()
            .copied()
            .filter(|target| !covered.contains(target))
            .collect();
        if uncovered.is_empty() {
            continue;
        }
        match lookup.is_published(&package.crate_name, &package.version) {
            Ok(true) => {}
            Ok(false) => out.push(format!(
                "warning: `{}@{}` is not published on crates.io, and has no release archive for {}; \
                 `gtc install` falls back to crates.io on those targets and fails there",
                package.crate_name,
                package.version,
                uncovered.join(", ")
            )),
            Err(err) => out.push(format!(
                "note: could not check `{}@{}` on crates.io ({err:#}); targets without an archive: {}",
                package.crate_name,
                package.version,
                uncovered.join(", ")
            )),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release_cmd::{PackageArtifactRef, ToolchainPackage};

    struct Fake(Vec<(&'static str, Result<bool, &'static str>)>);

    impl CratesIoVersionLookup for Fake {
        fn is_published(&self, crate_name: &str, _version: &str) -> Result<bool> {
            match self.0.iter().find(|(name, _)| *name == crate_name) {
                Some((_, Ok(found))) => Ok(*found),
                Some((_, Err(message))) => anyhow::bail!("{message}"),
                None => Ok(true),
            }
        }
    }

    fn package(name: &str, targets: &[&str]) -> ToolchainPackage {
        ToolchainPackage {
            crate_name: name.to_string(),
            bins: vec![name.to_string()],
            version: "1.2.3".to_string(),
            artifacts: Some(
                targets
                    .iter()
                    .map(|target| PackageArtifactRef {
                        target: (*target).to_string(),
                        url: format!("https://example.invalid/{name}-{target}.tgz"),
                        sha256: "00".repeat(32),
                    })
                    .collect(),
            ),
        }
    }

    fn manifest(packages: Vec<ToolchainPackage>) -> ToolchainManifest {
        let mut manifest: ToolchainManifest = serde_json::from_value(serde_json::json!({
            "schema": "greentic.toolchain-manifest.v1",
            "toolchain": "gtc",
            "version": "1.2.3",
            "channel": "dev",
            "created_at": "2026-10-05T00:00:00Z",
            "packages": []
        }))
        .expect("manifest skeleton parses");
        manifest.packages = packages;
        manifest
    }

    #[test]
    fn an_unpublished_pin_with_an_uncovered_target_is_reported_with_the_target() {
        let five: Vec<&str> = GTC_TARGETS
            .iter()
            .copied()
            .filter(|target| *target != "x86_64-apple-darwin")
            .collect();
        let warnings = fallback_warnings(
            &manifest(vec![package("greentic-pack-dev", &five)]),
            &Fake(vec![("greentic-pack-dev", Ok(false))]),
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("greentic-pack-dev@1.2.3"));
        assert!(warnings[0].contains("x86_64-apple-darwin"));
        assert!(warnings[0].starts_with("warning:"));
    }

    #[test]
    fn a_published_pin_is_not_reported() {
        let warnings = fallback_warnings(
            &manifest(vec![package("greentic-pack-dev", &[])]),
            &Fake(vec![("greentic-pack-dev", Ok(true))]),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn a_package_covering_every_target_never_asks_crates_io() {
        // A lookup that would fail loudly proves it was not consulted.
        let warnings = fallback_warnings(
            &manifest(vec![package("greentic-gui-dev", GTC_TARGETS)]),
            &Fake(vec![("greentic-gui-dev", Err("must not be called"))]),
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn a_failed_lookup_is_a_note_not_a_claim_that_the_version_is_missing() {
        let warnings = fallback_warnings(
            &manifest(vec![package("greentic-pack-dev", &[])]),
            &Fake(vec![("greentic-pack-dev", Err("timeout"))]),
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].starts_with("note:"));
        assert!(!warnings[0].contains("is not published"));
    }
}
