//! Resolve the dev toolchain from GitHub releases instead of crates.io.
//!
//! `release snapshot` pins every package to the newest dev version on
//! crates.io. On 2026-09-08 crates.io locked the account that publishes every
//! Greentic crate, and from then on every dev build still attached its archives
//! to a GitHub release while none of them reached the index — so the dev channel
//! froze on the versions it pinned the day before, with fixes merged and built
//! and unreachable through `gtc install`.
//!
//! `--source github-releases` reads the same builds from where they actually
//! landed. For each toolchain package it picks the newest release in the dev
//! lane whose archives are complete, and records each archive's URL and sha256
//! as the package's `artifacts`, which gtc installs directly instead of going
//! through `cargo binstall`.
//!
//! Two things crates.io used to do for us have to be done here instead:
//!
//! - **Yanking.** A GitHub release cannot be yanked. The 2026-09-06 worm
//!   rewrote branch tips with look-alike commits (LOCAL committer, a
//!   `.vscode/tasks.json` dropper, JavaScript posing as `public/fonts/fa-solid-*`
//!   files), and CI built releases from some of them. So the commit a release
//!   was tagged at must carry GitHub's own verified committer and none of those
//!   files. A release that fails that check is REFUSED, never skipped: quietly
//!   falling back to an older build would publish a channel nobody chose.
//! - **Lane filtering.** Only plain `MAJOR.MINOR.RUN` versions in the release's
//!   own lane count. greentic-pack also tags `v1.2.0-research.N` in the same
//!   minor, and those are not dev builds.

use anyhow::{Context, Result, bail};
use semver::Version;
use serde::Deserialize;

use crate::release_cmd::{
    PackageArtifactRef, TOOLCHAIN_MANIFEST_SCHEMA, TOOLCHAIN_NAME, ToolchainManifest,
    ToolchainPackage, ambient_github_token, check_release_assets, lane_of,
    manifest_bins_for_source, manifest_crate_name_for_source,
};
use crate::toolchain_catalogue::GREENTIC_TOOLCHAIN_PACKAGES;

const GITHUB_API_BASE: &str = "https://api.github.com";
const RELEASE_OWNER: &str = "greenticai";
const DEV_CHANNEL: &str = "dev";
/// Pages of 100 releases read before concluding a repository has no dev build.
/// Releases are listed newest first, so the current dev build is on page one in
/// practice; the extra pages only matter for a repository whose recent releases
/// are all on another lane.
const MAX_RELEASE_PAGES: u32 = 5;

/// One GitHub release, reduced to what resolution needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRelease {
    pub tag: String,
    pub draft: bool,
    pub assets: Vec<RepoReleaseAsset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoReleaseAsset {
    pub name: String,
    pub url: Option<String>,
    /// As GitHub reports it: `sha256:<hex>`.
    pub digest: Option<String>,
}

/// Where the commit a release is tagged at came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitProvenance {
    pub sha: String,
    pub committer: String,
    pub verified: bool,
    /// Every path in the commit's tree.
    pub paths: Vec<String>,
    /// GitHub truncates very large recursive trees; a truncated listing cannot
    /// prove a file is absent.
    pub tree_truncated: bool,
}

/// Reads releases and commit provenance. Injected so resolution is testable
/// without the network.
pub trait DevReleaseSource {
    /// Releases of `repo`, newest first, one page at a time (`page` starts at 1).
    fn releases_page(&self, repo: &str, page: u32) -> Result<Vec<RepoRelease>>;
    fn commit_provenance(&self, repo: &str, tag: &str) -> Result<CommitProvenance>;
}

/// A package resolved from its GitHub release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDevPackage {
    pub version: String,
    pub artifacts: Vec<PackageArtifactRef>,
}

/// Build a dev-channel manifest for gtc `release` from GitHub releases.
pub fn snapshot_manifest_from_github_releases(
    release: &str,
    source: &dyn DevReleaseSource,
    created_at: Option<String>,
) -> Result<ToolchainManifest> {
    let lane = lane_of(release).with_context(|| {
        format!("gtc release `{release}` is not MAJOR.MINOR.PATCH, so it names no lane")
    })?;
    let mut packages = Vec::new();
    for spec in GREENTIC_TOOLCHAIN_PACKAGES {
        let crate_in_manifest = manifest_crate_name_for_source(DEV_CHANNEL, spec.crate_name);
        let resolved = resolve_dev_package(source, spec.crate_name, &crate_in_manifest, lane)?;
        packages.push(ToolchainPackage {
            crate_name: crate_in_manifest,
            bins: manifest_bins_for_source(DEV_CHANNEL, spec.bins),
            version: resolved.version,
            artifacts: Some(resolved.artifacts),
        });
    }
    Ok(ToolchainManifest {
        schema: TOOLCHAIN_MANIFEST_SCHEMA.to_string(),
        toolchain: TOOLCHAIN_NAME.to_string(),
        version: release.to_string(),
        channel: Some(DEV_CHANNEL.to_string()),
        created_at,
        packages,
        extension_packs: None,
        components: None,
        gtc: None,
    })
}

/// Pick the release to pin for one package and name its archives.
///
/// `repo` is the base crate name (the repository), `crate_in_manifest` the
/// `-dev` mirror whose archives the release carries.
pub fn resolve_dev_package(
    source: &dyn DevReleaseSource,
    repo: &str,
    crate_in_manifest: &str,
    lane: (u64, u64),
) -> Result<ResolvedDevPackage> {
    let mut candidates: Vec<(Version, RepoRelease)> = Vec::new();
    for page in 1..=MAX_RELEASE_PAGES {
        let releases = source
            .releases_page(repo, page)
            .with_context(|| format!("failed to list releases of {RELEASE_OWNER}/{repo}"))?;
        if releases.is_empty() {
            break;
        }
        candidates.extend(releases.into_iter().filter_map(|release| {
            let version = dev_lane_version(&release, lane)?;
            Some((version, release))
        }));
        if !candidates.is_empty() {
            break;
        }
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0));

    let mut incomplete = Vec::new();
    for (version, release) in candidates {
        let version_text = version.to_string();
        let names: Vec<String> = release
            .assets
            .iter()
            .map(|asset| asset.name.clone())
            .collect();
        if let Err(reason) = check_release_assets(&names, &version_text) {
            // Still uploading, or a build that never finished: not a dev build
            // anyone can install yet, so the next one down is the newest that is.
            incomplete.push(format!("{}: {reason}", release.tag));
            continue;
        }
        let artifacts = artifacts_for(&release, crate_in_manifest, &version_text)?;
        let provenance = source
            .commit_provenance(repo, &release.tag)
            .with_context(|| format!("failed to read the commit behind {repo} {}", release.tag))?;
        refuse_unclean_commit(repo, &release.tag, &provenance)?;
        return Ok(ResolvedDevPackage {
            version: version_text,
            artifacts,
        });
    }
    if incomplete.is_empty() {
        bail!(
            "{RELEASE_OWNER}/{repo} has no {}.{}.<run> dev release",
            lane.0,
            lane.1
        );
    }
    bail!(
        "{RELEASE_OWNER}/{repo} has no complete {}.{}.<run> dev release:\n  {}",
        lane.0,
        lane.1,
        incomplete.join("\n  ")
    )
}

/// The version a release carries when it is a dev build in `lane`.
fn dev_lane_version(release: &RepoRelease, lane: (u64, u64)) -> Option<Version> {
    if release.draft {
        return None;
    }
    let version = Version::parse(release.tag.strip_prefix('v')?).ok()?;
    let in_lane = (version.major, version.minor) == lane;
    (in_lane && version.pre.is_empty() && version.build.is_empty()).then_some(version)
}

/// One artifact per target, from `<crate>-v<version>-<target>.tgz|.zip`.
fn artifacts_for(
    release: &RepoRelease,
    crate_in_manifest: &str,
    version: &str,
) -> Result<Vec<PackageArtifactRef>> {
    let prefix = format!("{crate_in_manifest}-v{version}-");
    let mut artifacts = Vec::new();
    for asset in &release.assets {
        let Some(rest) = asset.name.strip_prefix(&prefix) else {
            continue;
        };
        let Some(target) = rest
            .strip_suffix(".tgz")
            .or_else(|| rest.strip_suffix(".zip"))
        else {
            continue;
        };
        let (Some(url), Some(digest)) = (asset.url.as_deref(), asset.digest.as_deref()) else {
            bail!(
                "{} asset {} has no download URL or digest in the GitHub API response",
                release.tag,
                asset.name
            );
        };
        let Some(sha256) = digest.strip_prefix("sha256:") else {
            bail!(
                "{} asset {} reports a non-sha256 digest `{digest}`",
                release.tag,
                asset.name
            );
        };
        if artifacts
            .iter()
            .any(|existing: &PackageArtifactRef| existing.target == target)
        {
            bail!(
                "{} carries more than one {crate_in_manifest} archive for {target}",
                release.tag
            );
        }
        artifacts.push(PackageArtifactRef {
            target: target.to_string(),
            url: url.to_string(),
            sha256: sha256.to_string(),
        });
    }
    if artifacts.is_empty() {
        bail!(
            "{} carries no {crate_in_manifest}-v{version}-<target> archive",
            release.tag
        );
    }
    artifacts.sort_by(|left, right| left.target.cmp(&right.target));
    Ok(artifacts)
}

/// Paths the 2026-09-06 worm added to the commits it forged.
fn worm_marker(path: &str) -> bool {
    path == ".vscode/tasks.json"
        || path.ends_with("/.vscode/tasks.json")
        || path.contains("public/fonts/fa-solid-")
}

fn refuse_unclean_commit(repo: &str, tag: &str, provenance: &CommitProvenance) -> Result<()> {
    let mut problems = Vec::new();
    if provenance.committer != "GitHub" || !provenance.verified {
        problems.push(format!(
            "committer is `{}` (verified: {}), not GitHub's own verified committer",
            provenance.committer, provenance.verified
        ));
    }
    if provenance.tree_truncated {
        problems
            .push("the commit's tree listing is truncated, so it cannot be checked".to_string());
    }
    let markers: Vec<&str> = provenance
        .paths
        .iter()
        .map(String::as_str)
        .filter(|path| worm_marker(path))
        .collect();
    if !markers.is_empty() {
        problems.push(format!("the tree carries {}", markers.join(", ")));
    }
    if problems.is_empty() {
        return Ok(());
    }
    bail!(
        "refusing {RELEASE_OWNER}/{repo} {tag} (commit {}): {}. A GitHub release cannot be yanked, \
         so a build from a commit that does not look like one GitHub made is never pinned — \
         inspect it by hand and publish a clean build instead.",
        provenance.sha,
        problems.join("; ")
    )
}

/// Production source: the GitHub REST API, with the ambient token when one is
/// available (these repositories are public; the token lifts the rate limit).
pub struct GithubDevReleaseSource {
    base_url: String,
    token: Option<String>,
    client: reqwest::blocking::Client,
}

impl GithubDevReleaseSource {
    pub fn new(raw_token: Option<&str>) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(format!("greentic-dev/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build GitHub API client")?;
        Ok(Self {
            base_url: GITHUB_API_BASE.to_string(),
            token: ambient_github_token(raw_token),
            client,
        })
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let mut request = self
            .client
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json");
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .with_context(|| format!("failed to GET {url}"))?;
        let status = response.status();
        let body = response
            .text()
            .with_context(|| format!("failed to read body of {url}"))?;
        if !status.is_success() {
            bail!("GitHub API GET {url} returned {status}: {body}");
        }
        serde_json::from_str(&body).with_context(|| format!("failed to parse {url}"))
    }
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    #[serde(default)]
    browser_download_url: Option<String>,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Deserialize)]
struct ApiCommit {
    sha: String,
    commit: ApiCommitDetail,
}

#[derive(Deserialize)]
struct ApiCommitDetail {
    committer: ApiCommitter,
    #[serde(default)]
    verification: Option<ApiVerification>,
}

#[derive(Deserialize)]
struct ApiCommitter {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct ApiVerification {
    #[serde(default)]
    verified: bool,
}

#[derive(Deserialize)]
struct ApiTree {
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    tree: Vec<ApiTreeEntry>,
}

#[derive(Deserialize)]
struct ApiTreeEntry {
    path: String,
}

impl DevReleaseSource for GithubDevReleaseSource {
    fn releases_page(&self, repo: &str, page: u32) -> Result<Vec<RepoRelease>> {
        let releases: Vec<ApiRelease> = self.get_json(&format!(
            "/repos/{RELEASE_OWNER}/{repo}/releases?per_page=100&page={page}"
        ))?;
        Ok(releases
            .into_iter()
            .map(|release| RepoRelease {
                tag: release.tag_name,
                draft: release.draft,
                assets: release
                    .assets
                    .into_iter()
                    .map(|asset| RepoReleaseAsset {
                        name: asset.name,
                        url: asset.browser_download_url,
                        digest: asset.digest,
                    })
                    .collect(),
            })
            .collect())
    }

    fn commit_provenance(&self, repo: &str, tag: &str) -> Result<CommitProvenance> {
        let commit: ApiCommit =
            self.get_json(&format!("/repos/{RELEASE_OWNER}/{repo}/commits/{tag}"))?;
        let tree: ApiTree = self.get_json(&format!(
            "/repos/{RELEASE_OWNER}/{repo}/git/trees/{}?recursive=1",
            commit.sha
        ))?;
        Ok(CommitProvenance {
            sha: commit.sha,
            committer: commit.commit.committer.name,
            verified: commit
                .commit
                .verification
                .is_some_and(|verification| verification.verified),
            paths: tree.tree.into_iter().map(|entry| entry.path).collect(),
            tree_truncated: tree.truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const LANE: (u64, u64) = (1, 2);

    fn digest(ch: char) -> String {
        format!("sha256:{}", ch.to_string().repeat(64))
    }

    fn complete_release(crate_name: &str, version: &str) -> RepoRelease {
        let mut assets = Vec::new();
        for target in ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"] {
            let name = format!("{crate_name}-v{version}-{target}.tgz");
            assets.push(RepoReleaseAsset {
                url: Some(format!(
                    "https://github.com/greenticai/x/releases/download/v{version}/{name}"
                )),
                digest: Some(digest('a')),
                name: name.clone(),
            });
            assets.push(RepoReleaseAsset {
                name: format!("{name}.sha256"),
                url: Some("https://example.invalid/sidecar".to_string()),
                digest: Some(digest('b')),
            });
        }
        RepoRelease {
            tag: format!("v{version}"),
            draft: false,
            assets,
        }
    }

    fn clean() -> CommitProvenance {
        CommitProvenance {
            sha: "c".repeat(40),
            committer: "GitHub".to_string(),
            verified: true,
            paths: vec!["Cargo.toml".to_string(), "src/main.rs".to_string()],
            tree_truncated: false,
        }
    }

    struct FakeSource {
        releases: Vec<RepoRelease>,
        provenance: BTreeMap<String, CommitProvenance>,
    }

    impl DevReleaseSource for FakeSource {
        fn releases_page(&self, _repo: &str, page: u32) -> Result<Vec<RepoRelease>> {
            Ok(if page == 1 {
                self.releases.clone()
            } else {
                Vec::new()
            })
        }

        fn commit_provenance(&self, _repo: &str, tag: &str) -> Result<CommitProvenance> {
            Ok(self.provenance.get(tag).cloned().unwrap_or_else(clean))
        }
    }

    fn source(releases: Vec<RepoRelease>) -> FakeSource {
        FakeSource {
            releases,
            provenance: BTreeMap::new(),
        }
    }

    #[test]
    fn picks_the_newest_complete_dev_build_in_the_lane() {
        let crate_name = "greentic-start-dev";
        let mut uploading = complete_release(crate_name, "1.2.40000000000");
        uploading
            .assets
            .retain(|asset| !asset.name.ends_with(".sha256"));
        let mut draft = complete_release(crate_name, "1.2.50000000000");
        draft.draft = true;
        let fake = source(vec![
            complete_release(crate_name, "1.3.60000000000"),
            complete_release(crate_name, "1.2.0-research.4"),
            draft,
            uploading,
            complete_release(crate_name, "1.2.34207645334"),
            complete_release(crate_name, "1.2.34019234621"),
        ]);

        let resolved =
            resolve_dev_package(&fake, "greentic-start", crate_name, LANE).expect("resolved");

        assert_eq!(resolved.version, "1.2.34207645334");
        assert_eq!(
            resolved
                .artifacts
                .iter()
                .map(|artifact| artifact.target.as_str())
                .collect::<Vec<_>>(),
            vec!["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"]
        );
        assert!(
            resolved
                .artifacts
                .iter()
                .all(|artifact| artifact.sha256 == "a".repeat(64))
        );
        assert!(
            resolved
                .artifacts
                .iter()
                .all(|artifact| !artifact.url.ends_with(".sha256")),
            "a checksum sidecar is not an archive"
        );
    }

    #[test]
    fn refuses_rather_than_skips_a_release_built_from_a_forged_commit() {
        let crate_name = "greentic-runner-dev";
        let mut forged = clean();
        forged.committer = "greentic-ci[bot]".to_string();
        forged.verified = false;
        forged.paths.push(".vscode/tasks.json".to_string());
        forged
            .paths
            .push("public/fonts/fa-solid-400.woff2".to_string());
        let fake = FakeSource {
            releases: vec![
                complete_release(crate_name, "1.2.2"),
                complete_release(crate_name, "1.2.1"),
            ],
            provenance: BTreeMap::from([("v1.2.2".to_string(), forged)]),
        };

        let err = resolve_dev_package(&fake, "greentic-runner", crate_name, LANE)
            .expect_err("a forged newest build must not fall back to v1.2.1");
        let message = format!("{err:#}");
        assert!(message.contains("v1.2.2"), "{message}");
        assert!(message.contains("greentic-ci[bot]"), "{message}");
        assert!(message.contains(".vscode/tasks.json"), "{message}");
    }

    #[test]
    fn refuses_a_truncated_tree_it_cannot_check() {
        let crate_name = "greentic-mcp-dev";
        let mut truncated = clean();
        truncated.tree_truncated = true;
        let fake = FakeSource {
            releases: vec![complete_release(crate_name, "1.2.9")],
            provenance: BTreeMap::from([("v1.2.9".to_string(), truncated)]),
        };
        assert!(resolve_dev_package(&fake, "greentic-mcp", crate_name, LANE).is_err());
    }

    #[test]
    fn a_legitimate_vscode_settings_file_is_not_a_marker() {
        assert!(!worm_marker(".vscode/settings.json"));
        assert!(!worm_marker("web/public/fonts/inter.woff2"));
        assert!(worm_marker("crates/x/.vscode/tasks.json"));
        assert!(worm_marker("public/fonts/fa-solid-900.eot"));
    }

    #[test]
    fn an_archive_without_a_digest_is_an_error() {
        let crate_name = "greentic-pack-dev";
        let mut release = complete_release(crate_name, "1.2.7");
        release.assets[0].digest = None;
        let fake = source(vec![release]);
        let err = resolve_dev_package(&fake, "greentic-pack", crate_name, LANE)
            .expect_err("no digest, no pin");
        assert!(format!("{err:#}").contains("digest"), "{err:#}");
    }

    #[test]
    fn a_repository_with_no_dev_build_is_an_error() {
        let fake = source(vec![complete_release("greentic-flow-dev", "1.3.0")]);
        let err = resolve_dev_package(&fake, "greentic-flow", "greentic-flow-dev", LANE)
            .expect_err("no lane build");
        assert!(
            format!("{err:#}").contains("no 1.2.<run> dev release"),
            "{err:#}"
        );
    }

    #[test]
    fn only_incomplete_builds_report_why() {
        let mut uploading = complete_release("greentic-gui-dev", "1.2.5");
        uploading
            .assets
            .retain(|asset| !asset.name.ends_with(".sha256"));
        let fake = source(vec![uploading]);
        let err = resolve_dev_package(&fake, "greentic-gui", "greentic-gui-dev", LANE)
            .expect_err("incomplete");
        assert!(format!("{err:#}").contains("missing .sha256"), "{err:#}");
    }

    #[test]
    fn a_manifest_covers_every_toolchain_package_with_artifacts() {
        struct EveryRepo;
        impl DevReleaseSource for EveryRepo {
            fn releases_page(&self, repo: &str, page: u32) -> Result<Vec<RepoRelease>> {
                if page > 1 {
                    return Ok(Vec::new());
                }
                let crate_in_manifest = manifest_crate_name_for_source(DEV_CHANNEL, repo);
                Ok(vec![complete_release(&crate_in_manifest, "1.2.100")])
            }
            fn commit_provenance(&self, _repo: &str, _tag: &str) -> Result<CommitProvenance> {
                Ok(clean())
            }
        }

        let manifest = snapshot_manifest_from_github_releases("1.2.34087396714", &EveryRepo, None)
            .expect("manifest");

        assert_eq!(manifest.channel.as_deref(), Some("dev"));
        assert_eq!(manifest.packages.len(), GREENTIC_TOOLCHAIN_PACKAGES.len());
        for package in &manifest.packages {
            assert!(
                package.crate_name.ends_with("-dev"),
                "{}",
                package.crate_name
            );
            assert_eq!(package.version, "1.2.100");
            assert!(
                package.artifacts.as_ref().is_some_and(|a| !a.is_empty()),
                "{} has artifacts",
                package.crate_name
            );
        }
    }
}
