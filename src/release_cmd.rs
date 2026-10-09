use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use greentic_distributor_client::oci_client::Reference;
use greentic_distributor_client::oci_client::client::{
    Client, ClientConfig, ClientProtocol, Config, ImageLayer,
};
use greentic_distributor_client::oci_client::secrets::RegistryAuth;
use semver::Version;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::cli::{
    ReleaseGenerateArgs, ReleaseLatestArgs, ReleasePromoteArgs, ReleasePublishArgs,
    ReleaseSnapshotArgs, ReleaseViewArgs, SnapshotSource,
};
use crate::install::block_on_maybe_runtime;
use crate::passthrough::{ToolchainChannel, delegated_binary_name_for_channel};
use crate::toolchain_catalogue::{
    GREENTIC_COMPONENT_PACKAGES, GREENTIC_EXTENSION_PACK_PACKAGES, GREENTIC_TOOLCHAIN_PACKAGES,
    OciPackageSpec,
};

const DEFAULT_OAUTH_USER: &str = "oauth2";
pub const TOOLCHAIN_MANIFEST_SCHEMA: &str = "greentic.toolchain-manifest.v1";
pub const TOOLCHAIN_NAME: &str = "gtc";
pub const TOOLCHAIN_LAYER_MEDIA_TYPE: &str = "application/vnd.greentic.toolchain.manifest.v1+json";
const TOOLCHAIN_CONFIG_MEDIA_TYPE: &str = "application/vnd.greentic.toolchain.config.v1+json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolchainManifest {
    pub schema: String,
    pub toolchain: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    pub packages: Vec<ToolchainPackage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_packs: Option<Vec<ExtensionPackRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<ComponentRef>>,
    /// The gtc binary this manifest pins, named per target.
    ///
    /// gtc used to rebuild these names from the version using the STABLE
    /// convention (`gtc-<target>.tgz`) — while the dev lane publishes
    /// `gtc-dev-v<version>-<target>.tgz`. One convention in the consumer, two
    /// publishers: every dev self-update fetched a 404. Stating the name here
    /// removes the guess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gtc: Option<Vec<GtcArtifactRef>>,
}

/// One gtc release artifact, exactly as GitHub reports it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GtcArtifactRef {
    pub target: String,
    pub url: String,
    /// Hex sha256 without the `sha256:` prefix.
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolchainPackage {
    #[serde(rename = "crate")]
    pub crate_name: String,
    pub bins: Vec<String>,
    pub version: String,
    /// Release archives for this exact version, one per target. gtc installs
    /// from these instead of `cargo binstall` when the running target has an
    /// entry — which is what lets a manifest pin a build crates.io never
    /// received. Written by `release snapshot --source github-releases`; see
    /// `release_github_source`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<PackageArtifactRef>>,
}

/// One release archive of a toolchain package, exactly as GitHub reports it.
/// Same shape as [`GtcArtifactRef`], which gtc already consumes for self-update.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageArtifactRef {
    pub target: String,
    pub url: String,
    /// Hex sha256 without the `sha256:` prefix.
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtensionPackRef {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComponentRef {
    pub id: String,
    pub version: String,
}

pub fn generate(args: ReleaseGenerateArgs) -> Result<()> {
    let resolver = default_resolver();
    let artifact_resolver = GhcrArtifactVersionResolver::new(args.token.as_deref())?;
    let source = block_on_maybe_runtime(load_source_manifest(
        &args.repo,
        &args.from,
        args.token.as_deref(),
    ))
    .with_context(|| {
        format!(
            "failed to resolve source manifest `{}`",
            toolchain_ref(&args.repo, &args.from)
        )
    })?;
    let source = match source {
        Some(source) => Some(source),
        None => bootstrap_source_manifest_if_needed(
            &args.repo,
            &args.from,
            args.token.as_deref(),
            args.dry_run,
            &resolver,
        )?,
    };
    let manifest = generate_manifest_with_artifact_resolver(
        &args.release,
        &args.from,
        source.as_ref(),
        &resolver,
        &artifact_resolver,
        Some(created_at_now()?),
    )?;
    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&manifest)?);
        return Ok(());
    }
    let path = write_manifest(&args.out, &manifest)?;
    println!("Wrote {}", path.display());
    Ok(())
}

fn bootstrap_source_manifest_if_needed<R: CrateVersionResolver>(
    repo: &str,
    tag: &str,
    token: Option<&str>,
    dry_run: bool,
    resolver: &R,
) -> Result<Option<ToolchainManifest>> {
    let manifest = bootstrap_source_manifest(tag, resolver, Some(created_at_now()?))?;
    if dry_run {
        eprintln!(
            "Dry run: would bootstrap missing source manifest {}",
            toolchain_ref(repo, tag)
        );
        return Ok(Some(manifest));
    }

    let auth = match optional_registry_auth(token)? {
        RegistryAuth::Anonymous => {
            eprintln!(
                "Source manifest {} is missing; no GHCR token is available, so only the local release manifest will be generated.",
                toolchain_ref(repo, tag)
            );
            return Ok(Some(manifest));
        }
        auth => auth,
    };
    block_on_maybe_runtime(async {
        let client = oci_client();
        let source_ref = parse_reference(repo, tag)?;
        push_manifest_layer(&client, &source_ref, &auth, &manifest).await
    })
    .with_context(|| format!("failed to bootstrap {}", toolchain_ref(repo, tag)))?;
    println!("Bootstrapped {}", toolchain_ref(repo, tag));
    Ok(Some(manifest))
}

fn bootstrap_source_manifest<R: CrateVersionResolver>(
    tag: &str,
    resolver: &R,
    created_at: Option<String>,
) -> Result<ToolchainManifest> {
    generate_manifest(tag, tag, None, resolver, created_at)
}

pub fn publish(args: ReleasePublishArgs) -> Result<()> {
    let checker = default_release_checker(args.token.as_deref());
    publish_with_checker(args, &checker)
}

fn publish_with_checker(args: ReleasePublishArgs, checker: &dyn ReleaseAssetChecker) -> Result<()> {
    let (release, mut manifest, source) = publish_manifest_input(&args)?;

    // Gate before the dry-run return, so `publish --manifest <file> --dry-run`
    // doubles as the CI check on a pin bump: it answers "is this manifest
    // publishable?" without pushing anything.
    verify_manifest_releases(&manifest, args.tag.as_deref(), checker)?;

    // State the gtc artifacts rather than leaving the consumer to rebuild their
    // names. Deliberately NOT gated on the stable-channel check above: dev is
    // the lane whose names cannot be reconstructed, so it needs this most.
    // Best-effort — an unreadable release leaves the field absent and the
    // consumer falls back exactly as it does today. A manifest file that
    // already names them is left alone.
    if manifest.gtc.is_none() {
        manifest.gtc = gtc_artifacts_for(&manifest.version, checker)?;
    }

    if args.dry_run {
        println!(
            "Dry run: would publish {}",
            toolchain_ref(&args.repo, &release)
        );
        if let Some(tag) = &args.tag {
            println!(
                "Dry run: would tag {} as {}",
                toolchain_ref(&args.repo, &release),
                toolchain_ref(&args.repo, tag)
            );
        }
        return Ok(());
    }

    let auth = registry_auth(args.token.as_deref())?;
    block_on_maybe_runtime(async {
        let client = oci_client();
        let release_ref = parse_reference(&args.repo, &release)?;
        if !args.force && manifest_exists(&client, &release_ref, &auth).await? {
            bail!(
                "release tag `{}` already exists; pass --force to overwrite it",
                toolchain_ref(&args.repo, &release)
            );
        }
        push_manifest_layer(&client, &release_ref, &auth, &manifest).await?;
        if let Some(tag) = &args.tag {
            let tag_ref = parse_reference(&args.repo, tag)?;
            push_manifest_layer(&client, &tag_ref, &auth, &manifest).await?;
        }
        Ok(())
    })?;

    if let Some(source) = source {
        match source {
            PublishManifestSource::Generated(path) => println!("Wrote {}", path.display()),
            PublishManifestSource::Local(path) => println!("Read {}", path.display()),
        }
    }
    println!("Published {}", toolchain_ref(&args.repo, &release));
    if let Some(tag) = &args.tag {
        println!("Updated {}", toolchain_ref(&args.repo, tag));
    }

    if !args.no_notify_updater {
        let channel = manifest.channel.as_deref().unwrap_or("");
        notify_updater_dispatch(&release, channel, args.token.as_deref());
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PublishManifestSource {
    Generated(PathBuf),
    Local(PathBuf),
}

fn publish_manifest_input(
    args: &ReleasePublishArgs,
) -> Result<(String, ToolchainManifest, Option<PublishManifestSource>)> {
    if let Some(path) = &args.manifest {
        let mut manifest = read_manifest_file(path)?;
        validate_manifest(&manifest)?;
        let release = if let Some(release) = &args.release {
            manifest.version = release.clone();
            release.clone()
        } else {
            manifest.version.clone()
        };
        return Ok((
            release,
            manifest,
            Some(PublishManifestSource::Local(path.clone())),
        ));
    }

    let release = args
        .release
        .as_deref()
        .context("pass --release or --manifest")?;
    let from = args.from.as_deref().unwrap_or("latest");
    let resolver = default_resolver();
    let source = block_on_maybe_runtime(load_source_manifest(
        &args.repo,
        from,
        args.token.as_deref(),
    ))
    .with_context(|| {
        format!(
            "failed to resolve source manifest `{}`",
            toolchain_ref(&args.repo, from)
        )
    })?;
    if let Some(source_manifest) = source.as_ref()
        && source_manifest_has_concrete_pins(source_manifest)
    {
        eprintln!(
            "warning: `release publish --from {from}` reuses the pinned versions in `{}` instead \
             of querying crates.io. To refresh a channel from the latest crates.io versions, use \
             `release snapshot --channel <dev|research|stable>`. To copy an existing release tag without \
             re-resolving, use `release promote`. The conflated `--from` semantics will be \
             removed in a future release.",
            toolchain_ref(&args.repo, from),
        );
    }
    let manifest = generate_manifest(
        release,
        from,
        source.as_ref(),
        &resolver,
        Some(created_at_now()?),
    )?;
    let path = if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&manifest)?);
        None
    } else {
        Some(PublishManifestSource::Generated(write_manifest(
            &args.out, &manifest,
        )?))
    };
    Ok((release.to_string(), manifest, path))
}

/// True when the source manifest has at least one package pinned to a concrete
/// (non-`"latest"`) version. Used to detect the case where `release publish
/// --from <X>` would silently copy old pins instead of re-resolving — see the
/// deprecation warning emitted from `publish_manifest_input`.
fn source_manifest_has_concrete_pins(manifest: &ToolchainManifest) -> bool {
    manifest
        .packages
        .iter()
        .any(|package| package.version != "latest")
}

fn read_manifest_file(path: &Path) -> Result<ToolchainManifest> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn promote(args: ReleasePromoteArgs) -> Result<()> {
    if args.dry_run {
        println!(
            "Dry run: would promote {} to {}",
            toolchain_ref(&args.repo, &args.release),
            toolchain_ref(&args.repo, &args.tag)
        );
        return Ok(());
    }

    // Promote copies the OCI manifest (a tag alias) and never reads the
    // toolchain layer, so load it up front: the stable-lane gate needs the pins
    // to refuse moving `:stable` onto binaries that are not downloadable yet,
    // and the updater dispatch below needs the channel.
    let source = block_on_maybe_runtime(load_source_manifest(
        &args.repo,
        &args.release,
        args.token.as_deref(),
    ))
    .ok()
    .flatten();
    match source.as_ref() {
        Some(manifest) => {
            let checker = default_release_checker(args.token.as_deref());
            verify_manifest_releases(manifest, Some(args.tag.as_str()), &checker)?;
        }
        // Fail closed: moving the tag `gtc install` resolves without being able
        // to read its pins would reintroduce the exact hazard the gate exists
        // for. Other tags stay non-fatal, as before.
        None if args.tag == "stable" => bail!(
            "cannot read the toolchain pins of `{}` — refusing to move `:stable` unverified",
            toolchain_ref(&args.repo, &args.release)
        ),
        None => {}
    }

    let auth = registry_auth(args.token.as_deref())?;
    block_on_maybe_runtime(async {
        let client = oci_client();
        let source_ref = parse_reference(&args.repo, &args.release)?;
        let target_ref = parse_reference(&args.repo, &args.tag)?;
        let (manifest, _) = client
            .pull_manifest(&source_ref, &auth)
            .await
            .with_context(|| {
                format!(
                    "failed to resolve source release `{}`",
                    toolchain_ref(&args.repo, &args.release)
                )
            })?;
        client
            .push_manifest(&target_ref, &manifest)
            .await
            .with_context(|| {
                format!(
                    "failed to update tag `{}`",
                    toolchain_ref(&args.repo, &args.tag)
                )
            })?;
        Ok(())
    })?;
    println!(
        "Promoted {} to {}",
        toolchain_ref(&args.repo, &args.release),
        toolchain_ref(&args.repo, &args.tag)
    );

    if !args.no_notify_updater {
        // Reuses the manifest loaded before the push. If loading failed, skip
        // the dispatch — the workflow has its own channel guard as a backstop.
        let channel = source.and_then(|m| m.channel);
        match channel.as_deref() {
            Some(ch) => notify_updater_dispatch(&args.release, ch, args.token.as_deref()),
            None => {
                eprintln!(
                    "Skipping updater dispatch: could not determine channel \
                     for {} (use workflow_dispatch as fallback)",
                    toolchain_ref(&args.repo, &args.release)
                );
            }
        }
    }

    Ok(())
}

/// Snapshot the current crates.io state into a new toolchain manifest.
///
/// Unlike `publish --from <X>`, snapshot **never** reads an existing manifest
/// and **always** queries the resolver. That makes it safe to call repeatedly
/// to refresh a channel — `:dev` after each nightly publish, `:stable` after
/// a weekly release — without the promote-vs-snapshot conflation that bit
/// callers of `publish --from dev`.
pub fn snapshot(args: ReleaseSnapshotArgs) -> Result<()> {
    let checker = default_release_checker(args.token.as_deref());
    snapshot_with_checker(args, &checker)
}

fn snapshot_with_checker(
    args: ReleaseSnapshotArgs,
    checker: &dyn ReleaseAssetChecker,
) -> Result<()> {
    let channel = parse_channel(&args.channel)?;
    let mut manifest = match args.source {
        SnapshotSource::CratesIo => {
            let resolver = CratesIoApiVersionResolver::default();
            snapshot_manifest(&args.release, channel, &resolver, Some(created_at_now()?))?
        }
        SnapshotSource::GithubReleases => {
            // Only the dev lane publishes a GitHub release per build with the
            // `<crate>-dev-v<version>-<target>` archives this reads; stable and
            // research are released through crates.io, and pinning either from
            // here would name archives that do not exist.
            if channel != ToolchainChannel::Development {
                bail!(
                    "--source github-releases resolves the dev channel only, not `{}`",
                    args.channel
                );
            }
            let source =
                crate::release_github_source::GithubDevReleaseSource::new(args.token.as_deref())?;
            crate::release_github_source::snapshot_manifest_from_github_releases(
                &args.release,
                &source,
                Some(created_at_now()?),
            )?
        }
    };

    // Snapshot resolves pins from crates.io, which does NOT imply a finished
    // release build. Most repos gate `publish_crates` on `needs: [release]`, but
    // greentic-pack's `crates-publish.yml` fires independently on `push: tags:
    // ["v*"]` — so its crates.io version and its GitHub release race, and a
    // stable snapshot could pin the winner of that race. Same gate as publish.
    verify_manifest_releases(&manifest, args.tag.as_deref(), checker)?;

    if manifest.gtc.is_none() {
        manifest.gtc = gtc_artifacts_for(&manifest.version, checker)?;
    }

    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&manifest)?);
        println!(
            "Dry run: would publish {}",
            toolchain_ref(&args.repo, &args.release)
        );
        if let Some(tag) = &args.tag {
            println!(
                "Dry run: would tag {} as {}",
                toolchain_ref(&args.repo, &args.release),
                toolchain_ref(&args.repo, tag)
            );
        }
        return Ok(());
    }

    let path = write_manifest(&args.out, &manifest)?;
    println!("Wrote {}", path.display());

    let auth = registry_auth(args.token.as_deref())?;
    block_on_maybe_runtime(async {
        let client = oci_client();
        let release_ref = parse_reference(&args.repo, &args.release)?;
        if !args.force && manifest_exists(&client, &release_ref, &auth).await? {
            bail!(
                "release tag `{}` already exists; pass --force to overwrite it",
                toolchain_ref(&args.repo, &args.release)
            );
        }
        push_manifest_layer(&client, &release_ref, &auth, &manifest).await?;
        if let Some(tag) = &args.tag {
            let tag_ref = parse_reference(&args.repo, tag)?;
            push_manifest_layer(&client, &tag_ref, &auth, &manifest).await?;
        }
        Ok(())
    })?;
    println!("Published {}", toolchain_ref(&args.repo, &args.release));
    if let Some(tag) = &args.tag {
        println!("Updated {}", toolchain_ref(&args.repo, tag));
    }

    if !args.no_notify_updater {
        let ch = channel_tag(channel);
        notify_updater_dispatch(&args.release, ch, args.token.as_deref());
    }

    Ok(())
}

fn parse_channel(channel: &str) -> Result<ToolchainChannel> {
    match channel {
        "dev" | "development" => Ok(ToolchainChannel::Development),
        "rnd" | "research" => Ok(ToolchainChannel::Rnd),
        "stable" => Ok(ToolchainChannel::Stable),
        other => bail!(
            "unknown channel `{other}` (expected `dev`, `research` (alias `rnd`), or `stable`); \
             pass --channel dev for the dev lane, --channel research for the research lane, or \
             --channel stable for the stable lane"
        ),
    }
}

fn channel_tag(channel: ToolchainChannel) -> &'static str {
    match channel {
        ToolchainChannel::Stable => "stable",
        ToolchainChannel::Development => "dev",
        ToolchainChannel::Rnd => "rnd",
    }
}

/// Resolve the version for a single toolchain manifest entry on `channel`.
/// Returns `Ok(None)` when the research channel has no `-rnd` build for the
/// crate (skip it) so manifest assembly does not abort on the ~10 of 13
/// toolchain crates that ship no research build.
fn resolve_manifest_version<R: CrateVersionResolver>(
    resolver: &R,
    crate_in_manifest: &str,
    channel: ToolchainChannel,
    lane: Option<(u64, u64)>,
) -> Result<Option<String>> {
    if channel == ToolchainChannel::Rnd {
        match resolver
            .resolve_research_version(crate_in_manifest)
            .with_context(|| {
                format!("failed to resolve research version for `{crate_in_manifest}`")
            })? {
            ResearchVersion::Pinned(version) => Ok(Some(version)),
            ResearchVersion::Absent => {
                eprintln!(
                    "note: `{crate_in_manifest}` has no research build on crates.io; \
                     omitting it from the research toolchain manifest"
                );
                Ok(None)
            }
        }
    } else if let (ToolchainChannel::Development, Some(lane)) = (channel, lane) {
        // Stay inside the release's own minor line. Without this the dev
        // manifest pins whatever sorts highest across ALL lanes, which is how
        // an abandoned 1.3 research build kept winning over active 1.2 dev
        // builds and froze the dev channel.
        resolver
            .resolve_latest_in_lane(crate_in_manifest, lane)
            .with_context(|| {
                format!(
                    "failed to resolve a {}.{} version for `{crate_in_manifest}`",
                    lane.0, lane.1
                )
            })
            .map(Some)
    } else {
        resolver
            .resolve_latest_for_channel(crate_in_manifest, channel)
            .with_context(|| format!("failed to resolve latest version for `{crate_in_manifest}`"))
            .map(Some)
    }
}

pub fn snapshot_manifest<R: CrateVersionResolver>(
    release: &str,
    channel: ToolchainChannel,
    resolver: &R,
    created_at: Option<String>,
) -> Result<ToolchainManifest> {
    let from = channel_tag(channel);
    let mut packages = Vec::new();
    for package in GREENTIC_TOOLCHAIN_PACKAGES {
        let crate_in_manifest = manifest_crate_name_for_source(from, package.crate_name);
        let Some(version) =
            resolve_manifest_version(resolver, &crate_in_manifest, channel, lane_of(release))?
        else {
            continue;
        };
        packages.push(ToolchainPackage {
            crate_name: crate_in_manifest,
            bins: manifest_bins_for_source(from, package.bins),
            version,
            artifacts: None,
        });
    }
    Ok(ToolchainManifest {
        schema: TOOLCHAIN_MANIFEST_SCHEMA.to_string(),
        toolchain: TOOLCHAIN_NAME.to_string(),
        version: release.to_string(),
        channel: Some(from.to_string()),
        created_at,
        packages,
        extension_packs: None,
        components: None,
        gtc: None,
    })
}

pub fn view(args: ReleaseViewArgs) -> Result<()> {
    let tag = release_view_tag(&args)?;
    let manifest = block_on_maybe_runtime(load_source_manifest(
        &args.repo,
        &tag,
        args.token.as_deref(),
    ))
    .with_context(|| {
        format!(
            "failed to resolve manifest `{}`",
            toolchain_ref(&args.repo, &tag)
        )
    })?
    .with_context(|| {
        format!(
            "manifest `{}` was not found or is not authorized for this token",
            toolchain_ref(&args.repo, &tag)
        )
    })?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}

pub fn latest(args: ReleaseLatestArgs) -> Result<()> {
    let manifest = latest_manifest(Some(created_at_now()?));
    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&manifest)?);
        println!(
            "Dry run: would publish {}",
            toolchain_ref(&args.repo, "latest")
        );
        return Ok(());
    }

    let auth = registry_auth(args.token.as_deref())?;
    block_on_maybe_runtime(async {
        let client = oci_client();
        let latest_ref = parse_reference(&args.repo, "latest")?;
        if !args.force && manifest_exists(&client, &latest_ref, &auth).await? {
            bail!(
                "latest tag `{}` already exists; pass --force to overwrite it",
                toolchain_ref(&args.repo, "latest")
            );
        }
        push_manifest_layer(&client, &latest_ref, &auth, &manifest).await
    })?;
    println!("Published {}", toolchain_ref(&args.repo, "latest"));
    Ok(())
}

fn latest_manifest(created_at: Option<String>) -> ToolchainManifest {
    ToolchainManifest {
        schema: TOOLCHAIN_MANIFEST_SCHEMA.to_string(),
        toolchain: TOOLCHAIN_NAME.to_string(),
        version: "latest".to_string(),
        channel: Some("latest".to_string()),
        created_at,
        packages: latest_manifest_packages(),
        extension_packs: Some(
            GREENTIC_EXTENSION_PACK_PACKAGES
                .iter()
                .map(|package| ExtensionPackRef {
                    id: package.package.to_string(),
                    version: "latest".to_string(),
                })
                .collect(),
        ),
        components: Some(
            GREENTIC_COMPONENT_PACKAGES
                .iter()
                .map(|package| ComponentRef {
                    id: package.package.to_string(),
                    version: "latest".to_string(),
                })
                .collect(),
        ),
        gtc: None,
    }
}

fn latest_manifest_packages() -> Vec<ToolchainPackage> {
    std::iter::once(ToolchainPackage {
        crate_name: delegated_binary_name_for_channel(
            TOOLCHAIN_NAME,
            ToolchainChannel::Development,
        ),
        bins: vec![delegated_binary_name_for_channel(
            TOOLCHAIN_NAME,
            ToolchainChannel::Development,
        )],
        version: "latest".to_string(),
        artifacts: None,
    })
    .chain(GREENTIC_TOOLCHAIN_PACKAGES.iter().map(|package| {
        ToolchainPackage {
            crate_name: delegated_binary_name_for_channel(
                package.crate_name,
                ToolchainChannel::Development,
            ),
            bins: package
                .bins
                .iter()
                .map(|bin| delegated_binary_name_for_channel(bin, ToolchainChannel::Development))
                .collect(),
            version: "latest".to_string(),
            artifacts: None,
        }
    }))
    .collect()
}

fn release_view_tag(args: &ReleaseViewArgs) -> Result<String> {
    match (&args.release, &args.tag) {
        (Some(release), None) => Ok(release.clone()),
        (None, Some(tag)) => Ok(tag.clone()),
        _ => bail!("pass exactly one of --release or --tag"),
    }
}

pub fn generate_manifest<R: CrateVersionResolver>(
    release: &str,
    from: &str,
    source: Option<&ToolchainManifest>,
    resolver: &R,
    created_at: Option<String>,
) -> Result<ToolchainManifest> {
    let artifact_resolver = ReleaseArtifactVersionResolver { release };
    generate_manifest_with_artifact_resolver(
        release,
        from,
        source,
        resolver,
        &artifact_resolver,
        created_at,
    )
}

pub fn generate_manifest_with_artifact_resolver<R, A>(
    release: &str,
    from: &str,
    source: Option<&ToolchainManifest>,
    resolver: &R,
    artifact_resolver: &A,
    created_at: Option<String>,
) -> Result<ToolchainManifest>
where
    R: CrateVersionResolver,
    A: ArtifactVersionResolver,
{
    if let Some(source) = source {
        validate_manifest(source)?;
    }
    let source_versions = source_version_map(source);
    let mut packages = Vec::new();
    for package in GREENTIC_TOOLCHAIN_PACKAGES {
        let crate_in_manifest = manifest_crate_name_for_source(from, package.crate_name);
        let source_version = source_versions.get(&crate_in_manifest);
        let version = match source_version.map(String::as_str) {
            Some(version) if version != "latest" => Some(version.to_string()),
            _ => resolve_manifest_version(
                resolver,
                &crate_in_manifest,
                channel_from_source_tag(from),
                lane_of(release),
            )?,
        };
        let Some(version) = version else {
            continue;
        };
        packages.push(ToolchainPackage {
            crate_name: crate_in_manifest,
            bins: manifest_bins_for_source(from, package.bins),
            version,
            artifacts: None,
        });
    }
    Ok(ToolchainManifest {
        schema: TOOLCHAIN_MANIFEST_SCHEMA.to_string(),
        toolchain: TOOLCHAIN_NAME.to_string(),
        version: release.to_string(),
        channel: Some(from.to_string()),
        created_at,
        packages,
        extension_packs: Some(extension_pack_refs_for_release(source, artifact_resolver)?),
        components: Some(component_refs_for_release(source, artifact_resolver)?),
        gtc: None,
    })
}

/// Map a manifest source-tag (`dev`/`rnd`/`stable`) to its channel.
fn channel_from_source_tag(from: &str) -> ToolchainChannel {
    match from {
        "dev" => ToolchainChannel::Development,
        "rnd" => ToolchainChannel::Rnd,
        _ => ToolchainChannel::Stable,
    }
}

pub(crate) fn manifest_bins_for_source(from: &str, bins: &[&str]) -> Vec<String> {
    let channel = channel_from_source_tag(from);
    bins.iter()
        .map(|bin| delegated_binary_name_for_channel(bin, channel))
        .collect()
}

fn extension_pack_refs_for_release<A: ArtifactVersionResolver>(
    source: Option<&ToolchainManifest>,
    artifact_resolver: &A,
) -> Result<Vec<ExtensionPackRef>> {
    let source_versions = source_ref_version_map(source.and_then(|manifest| {
        manifest
            .extension_packs
            .as_ref()
            .map(|refs| refs.iter().map(|item| (&item.id, &item.version)))
    }));
    GREENTIC_EXTENSION_PACK_PACKAGES
        .iter()
        .map(|package| {
            Ok(ExtensionPackRef {
                id: package.package.to_string(),
                version: ref_version_for_package(package, &source_versions, artifact_resolver)?,
            })
        })
        .collect()
}

fn component_refs_for_release<A: ArtifactVersionResolver>(
    source: Option<&ToolchainManifest>,
    artifact_resolver: &A,
) -> Result<Vec<ComponentRef>> {
    let source_versions = source_ref_version_map(source.and_then(|manifest| {
        manifest
            .components
            .as_ref()
            .map(|refs| refs.iter().map(|item| (&item.id, &item.version)))
    }));
    GREENTIC_COMPONENT_PACKAGES
        .iter()
        .map(|package| {
            Ok(ComponentRef {
                id: package.package.to_string(),
                version: ref_version_for_package(package, &source_versions, artifact_resolver)?,
            })
        })
        .collect()
}

fn source_ref_version_map<'a, I>(refs: Option<I>) -> BTreeMap<String, String>
where
    I: Iterator<Item = (&'a String, &'a String)>,
{
    let mut out = BTreeMap::new();
    if let Some(refs) = refs {
        for (id, version) in refs {
            out.insert(id.clone(), version.clone());
        }
    }
    out
}

fn ref_version_for_package(
    package: &OciPackageSpec,
    source_versions: &BTreeMap<String, String>,
    artifact_resolver: &impl ArtifactVersionResolver,
) -> Result<String> {
    match source_versions.get(package.package).map(String::as_str) {
        Some(version) if version != "latest" => Ok(version.to_string()),
        _ => artifact_resolver
            .resolve_latest(package.package)
            .with_context(|| format!("failed to resolve GHCR version for `{}`", package.package)),
    }
}

/// Apply the dev-channel `-dev` suffix to a crate name when the manifest
/// channel is `"dev"`. The dev-publish lane mirrors every binary crate as
/// `<crate>-dev` (binary bifurcation); the toolchain manifest must pin the
/// mirrored crate so `cargo binstall` resolves the dev artifact instead of
/// the stable one. Reuses `delegated_binary_name_for_channel` because the
/// rule is identical for crates and binaries (`-dev` suffix, with the
/// special carve-out that `greentic-dev` itself becomes `greentic-dev-dev`).
pub(crate) fn manifest_crate_name_for_source(from: &str, crate_name: &str) -> String {
    if from == "dev" {
        delegated_binary_name_for_channel(crate_name, ToolchainChannel::Development)
    } else {
        crate_name.to_string()
    }
}

pub fn validate_manifest(manifest: &ToolchainManifest) -> Result<()> {
    if manifest.schema != TOOLCHAIN_MANIFEST_SCHEMA {
        bail!(
            "unsupported toolchain manifest schema `{}`",
            manifest.schema
        );
    }
    if manifest.toolchain != TOOLCHAIN_NAME {
        bail!("unsupported toolchain `{}`", manifest.toolchain);
    }
    Ok(())
}

pub fn toolchain_ref(repo: &str, tag: &str) -> String {
    format!("{repo}:{tag}")
}

// ---------------------------------------------------------------------------
// Stable-lane release gate — a published manifest must never pin a version
// whose binaries are not downloadable yet
// ---------------------------------------------------------------------------

const GITHUB_API_BASE: &str = "https://api.github.com";
/// Every toolchain package's crate name doubles as its repo name under this org.
const TOOLCHAIN_RELEASE_OWNER: &str = "greenticai";
/// Archive extensions the shared `release-binaries.yml` workflow attaches.
const RELEASE_ARCHIVE_SUFFIXES: [&str; 2] = [".tgz", ".zip"];

/// Reads the asset names of one package's GitHub release. Injected so the gate
/// is testable without a network round-trip, mirroring [`CrateVersionResolver`].
trait ReleaseAssetChecker {
    /// `Ok(None)` when the release does not exist, `Ok(Some(names))` otherwise.
    fn release_assets(&self, repo: &str, tag: &str) -> Result<Option<Vec<String>>>;

    /// The same release, with each asset's URL and digest.
    ///
    /// Defaulted to `None` so the existing test doubles — which model asset
    /// NAMES only, because that is all the publish gate ever needed — keep
    /// compiling and keep exercising that gate unchanged.
    fn release_artifacts(&self, _repo: &str, _tag: &str) -> Result<Option<Vec<ReleaseArtifact>>> {
        Ok(None)
    }
}

#[derive(Deserialize)]
struct GithubReleaseAssets {
    #[serde(default)]
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Deserialize)]
struct GithubReleaseAsset {
    name: String,
    /// GitHub reports `sha256:<hex>`; absent on older releases.
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    browser_download_url: Option<String>,
}

/// A release asset with the download URL and digest GitHub itself reports —
/// so nothing downstream has to reconstruct either.
pub(crate) struct ReleaseArtifact {
    pub name: String,
    pub url: Option<String>,
    pub sha256: Option<String>,
}

struct GithubReleaseAssetChecker {
    base_url: String,
    token: Option<String>,
    client: reqwest::blocking::Client,
}

impl GithubReleaseAssetChecker {
    fn new(base_url: impl Into<String>, token: Option<String>) -> Self {
        let client = reqwest::blocking::Client::builder()
            .user_agent(format!("greentic-dev/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build GitHub API client");
        Self {
            base_url: base_url.into(),
            token,
            client,
        }
    }
}

impl GithubReleaseAssetChecker {
    /// One GET, shared by both trait methods.
    fn fetch_assets(&self, repo: &str, tag: &str) -> Result<Option<Vec<GithubReleaseAsset>>> {
        let url = format!(
            "{}/repos/{TOOLCHAIN_RELEASE_OWNER}/{repo}/releases/tags/{tag}",
            self.base_url.trim_end_matches('/')
        );
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
        let Some(body) = classify_release_response(status, &url, body)? else {
            return Ok(None);
        };
        let release: GithubReleaseAssets = serde_json::from_str(&body)
            .with_context(|| format!("failed to parse release metadata from {url}"))?;
        Ok(Some(release.assets))
    }
}

impl ReleaseAssetChecker for GithubReleaseAssetChecker {
    fn release_assets(&self, repo: &str, tag: &str) -> Result<Option<Vec<String>>> {
        Ok(self
            .fetch_assets(repo, tag)?
            .map(|assets| assets.into_iter().map(|asset| asset.name).collect()))
    }

    fn release_artifacts(&self, repo: &str, tag: &str) -> Result<Option<Vec<ReleaseArtifact>>> {
        Ok(self.fetch_assets(repo, tag)?.map(|assets| {
            assets
                .into_iter()
                .map(|asset| ReleaseArtifact {
                    name: asset.name,
                    url: asset.browser_download_url,
                    sha256: asset.digest,
                })
                .collect()
        }))
    }
}

/// The gate's production checker: real GitHub API, ambient token when one is
/// available. Release reads on these public repos work unauthenticated; the
/// token only lifts the rate limit.
fn default_release_checker(raw_token: Option<&str>) -> GithubReleaseAssetChecker {
    GithubReleaseAssetChecker::new(GITHUB_API_BASE, ambient_github_token(raw_token))
}

/// Classify a GitHub release-by-tag response: `Ok(None)` for a 404 (no such
/// release), `Ok(Some(body))` on success, `Err` otherwise. Pure so the
/// 404-vs-error decision is unit-testable without a live HTTP round-trip.
fn classify_release_response(
    status: reqwest::StatusCode,
    url: &str,
    body: String,
) -> Result<Option<String>> {
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        bail!("GitHub API GET {url} returned {status}: {body}");
    }
    Ok(Some(body))
}

/// True when a push affects what `gtc install` resolves: either the manifest
/// declares the stable channel, or the push moves the `stable` tag itself.
///
/// The tag half is not redundant. `generate_manifest` records `channel: <the
/// --from value>`, and `--from` defaults to `latest`, so `publish --tag stable`
/// routinely ships a manifest whose channel is `"latest"` while still moving the
/// tag users install from. Gating on the channel alone would wave it through.
fn affects_stable_channel(manifest: &ToolchainManifest, target_tag: Option<&str>) -> bool {
    manifest.channel.as_deref() == Some("stable") || target_tag == Some("stable")
}

/// Refuse to publish a stable-lane manifest that pins a package version whose
/// GitHub release is missing or still uploading.
///
/// The toolchain manifest is what `gtc install` resolves, so a pin that outruns
/// its release build leaves `:stable` pointing at binaries nobody can download.
/// The dev and research lanes publish on their own cadence and are never gated.
/// gtc lives in its own repository, not one named after a pinned crate, so it
/// is absent from `manifest.packages` and needs its own lookup.
const GTC_RELEASE_REPO: &str = "greentic";

/// Every target gtc is ever built for. A lane that builds a subset simply has
/// no asset for the rest, and those are skipped — the manifest states what was
/// actually published, never what should have been.
const GTC_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
];

/// Name the gtc artifacts for `version`, straight from the release.
///
/// `Ok(None)` when the release cannot be read or names nothing usable: the
/// manifest then carries no `gtc` field and the consumer falls back to
/// reconstruction, which is what every manifest published so far does.
fn gtc_artifacts_for(
    version: &str,
    checker: &dyn ReleaseAssetChecker,
) -> Result<Option<Vec<GtcArtifactRef>>> {
    let tag = format!("v{version}");
    let Some(artifacts) = checker.release_artifacts(GTC_RELEASE_REPO, &tag)? else {
        return Ok(None);
    };
    let mut named = Vec::new();
    for target in GTC_TARGETS {
        let tgz = format!("-{target}.tgz");
        let zip = format!("-{target}.zip");
        let Some(found) = artifacts
            .iter()
            .find(|artifact| artifact.name.ends_with(&tgz) || artifact.name.ends_with(&zip))
        else {
            continue;
        };
        let (Some(url), Some(digest)) = (found.url.as_deref(), found.sha256.as_deref()) else {
            continue;
        };
        named.push(GtcArtifactRef {
            target: (*target).to_string(),
            url: url.to_string(),
            sha256: digest.trim_start_matches("sha256:").to_string(),
        });
    }
    Ok((!named.is_empty()).then_some(named))
}

fn verify_manifest_releases(
    manifest: &ToolchainManifest,
    target_tag: Option<&str>,
    checker: &dyn ReleaseAssetChecker,
) -> Result<()> {
    if !affects_stable_channel(manifest, target_tag) {
        return Ok(());
    }
    let mut problems = Vec::new();
    for package in &manifest.packages {
        let tag = format!("v{}", package.version);
        match checker.release_assets(&package.crate_name, &tag)? {
            None => problems.push(format!(
                "{} {tag}: no GitHub release (build not finished)",
                package.crate_name
            )),
            Some(assets) => {
                if let Err(reason) = check_release_assets(&assets, &package.version) {
                    problems.push(format!("{} {tag}: {reason}", package.crate_name));
                }
            }
        }
    }
    if !problems.is_empty() {
        bail!(
            "refusing to publish toolchain manifest {}: {} pinned package(s) are not \
             downloadable yet:\n  {}\nWait for the release builds to finish, then retry.",
            manifest.version,
            problems.len(),
            problems.join("\n  ")
        );
    }
    Ok(())
}

/// A release is usable once it carries at least one versioned archive and every
/// versioned archive has its `.sha256` sibling. The `ensure-release` action
/// creates the release and *then* uploads the assets, so a half-populated
/// release is an observed state rather than a theoretical one.
///
/// Only assets embedding `-v<version>-` count. `greentic-pack` also attaches
/// unversioned `greentic-pack-<target>.tgz` aliases that carry no checksums —
/// deliberate `binstall` shims, not a half-finished upload — and demanding a
/// `.sha256` for those would reject every pack release ever published.
///
/// KNOWN LIMITATION: this cannot detect a release whose upload is partway
/// through its *first* target, because it has no notion of the expected target
/// matrix. Nothing available makes that knowable cheaply — `ensure-release`
/// creates the release without `--draft` (so there is no atomic publish
/// marker), the target set varies per package (`include-macos-intel`), and the
/// manifest's `bins` holds the delegated binary name, not the archive prefix
/// (greentic-mcp declares `greentic-mcp` but ships `greentic-mcp-exec-*` and
/// `greentic-mcp-generator-*`), so it cannot drive a per-binary check either.
/// The residual window is one `gh release upload` invocation — all assets go up
/// in a single call — against the 35-45 minutes of build time this does cover.
pub(crate) fn check_release_assets(assets: &[String], version: &str) -> Result<(), String> {
    let names: BTreeSet<&str> = assets.iter().map(String::as_str).collect();
    let version_marker = format!("-v{version}-");
    let archives: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| {
            name.contains(&version_marker)
                && RELEASE_ARCHIVE_SUFFIXES
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
        })
        .collect();
    if archives.is_empty() {
        return Err(format!(
            "release has no v{version} archives yet (upload in progress)"
        ));
    }
    let unchecksummed: Vec<&str> = archives
        .iter()
        .copied()
        .filter(|name| !names.contains(format!("{name}.sha256").as_str()))
        .collect();
    if !unchecksummed.is_empty() {
        return Err(format!(
            "archives missing .sha256 (upload in progress): {}",
            unchecksummed.join(", ")
        ));
    }
    Ok(())
}

fn source_version_map(source: Option<&ToolchainManifest>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(source) = source {
        for package in &source.packages {
            out.insert(package.crate_name.clone(), package.version.clone());
        }
    }
    out
}

fn write_manifest(out_dir: &Path, manifest: &ToolchainManifest) -> Result<PathBuf> {
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;
    let path = out_dir.join(manifest_file_name(manifest));
    let json = serde_json::to_vec_pretty(manifest).context("failed to serialize manifest")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

fn manifest_file_name(manifest: &ToolchainManifest) -> String {
    match manifest.channel.as_deref() {
        Some("stable") | None => format!("gtc-{}.json", manifest.version),
        Some(channel) => format!("gtc-{channel}-{}.json", manifest.version),
    }
}

fn created_at_now() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("failed to format current time")
}

/// Outcome of resolving the research (`-rnd`) version of a toolchain crate.
///
/// Only `start`/`runner`/`setup` carry `-research` builds; the other ~10
/// delegated toolchain crates have no `<name>-rnd` published. Resolving those
/// must not be a hard error — it is an expected "no research build" signal that
/// the caller turns into a skip, so the research channel still assembles.
pub enum ResearchVersion {
    /// The `<name>-rnd` crate is published; pin this exact version.
    Pinned(String),
    /// The `<name>-rnd` crate is not published on crates.io (HTTP 404). The
    /// tool ships no research build and must be skipped on the research channel
    /// rather than aborting the whole install / manifest assembly.
    Absent,
}

pub trait CrateVersionResolver {
    fn resolve_latest(&self, crate_name: &str) -> Result<String>;

    /// Channel-aware resolution. The research (`rnd`) lane publishes base-name
    /// crates at `X.Y.Z-research` PRERELEASES (greentic-runner's
    /// research-publish.yml), which `resolve_latest`'s `max_stable_version`
    /// preference silently skips — so the dev/stable behaviour returns the old
    /// stable (e.g. `0.5.x`) instead of the current `1.2.0-research`. The
    /// default delegates to `resolve_latest` (correct for dev/stable).
    fn resolve_latest_for_channel(
        &self,
        crate_name: &str,
        _channel: ToolchainChannel,
    ) -> Result<String> {
        self.resolve_latest(crate_name)
    }

    /// Resolve the latest version INSIDE a `(major, minor)` lane.
    ///
    /// The dev channel needs this: greentic versions its lanes by minor (1.2.x
    /// dev, 1.3.x research), so "highest overall" lets an abandoned research
    /// build outrank an active dev one. The default ignores the lane, which is
    /// correct for resolvers that serve a single lane (the test fakes).
    fn resolve_latest_in_lane(&self, crate_name: &str, _lane: (u64, u64)) -> Result<String> {
        self.resolve_latest(crate_name)
    }

    /// Resolve the research (`-rnd`) version, distinguishing an unpublished
    /// crate (HTTP 404 → [`ResearchVersion::Absent`]) from a genuine resolution
    /// error. The default treats every resolvable crate as
    /// [`ResearchVersion::Pinned`]; only the crates.io resolver can observe a
    /// 404, so it overrides this.
    fn resolve_research_version(&self, crate_name: &str) -> Result<ResearchVersion> {
        self.resolve_latest_for_channel(crate_name, ToolchainChannel::Rnd)
            .map(ResearchVersion::Pinned)
    }
}

/// Default resolver used by `generate`, `publish`, and `snapshot`. Hits the
/// crates.io HTTP API directly — see `CratesIoApiVersionResolver` for why
/// this is preferred over shelling out to `cargo search`.
fn default_resolver() -> CratesIoApiVersionResolver {
    CratesIoApiVersionResolver::default()
}

pub trait ArtifactVersionResolver {
    fn resolve_latest(&self, package: &str) -> Result<String>;
}

const CRATES_IO_API_BASE: &str = "https://crates.io/api/v1/crates";
const CRATES_IO_USER_AGENT: &str = concat!(
    "greentic-dev/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/greenticai/greentic-dev)"
);

/// Resolve the latest published version of a crate by hitting the crates.io
/// HTTP API directly. Returns `max_stable_version` when present, falling back
/// to `newest_version` and then `max_version`. Replaces an earlier
/// `cargo search`-based resolver that ranked results by relevance and parsed
/// stdout heuristically — both brittle for `<name>-dev` aliases that share
/// prefixes with their stable parents.
pub struct CratesIoApiVersionResolver {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl Default for CratesIoApiVersionResolver {
    fn default() -> Self {
        Self::new(CRATES_IO_API_BASE)
    }
}

impl CratesIoApiVersionResolver {
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = reqwest::blocking::Client::builder()
            .user_agent(CRATES_IO_USER_AGENT)
            .build()
            .expect("failed to build crates.io API client");
        Self {
            base_url: base_url.into(),
            client,
        }
    }

    /// GET the crates.io page for `crate_name`. `Ok(None)` when the crate is
    /// absent (HTTP 404), `Ok(Some(body))` on success, `Err` on any other
    /// status or transport failure. Lets callers treat "no such crate" as a
    /// skip rather than a hard error.
    fn fetch_crate_body(&self, crate_name: &str) -> Result<Option<String>> {
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), crate_name);
        let response = self
            .client
            .get(&url)
            .send()
            .with_context(|| format!("failed to GET {url}"))?;
        let status = response.status();
        let body = response
            .text()
            .with_context(|| format!("failed to read body of {url}"))?;
        classify_crate_response(status, &url, body)
    }
}

/// Classify a crates.io crate-page response by HTTP status: `Ok(None)` for a
/// 404 (crate absent), `Ok(Some(body))` for success, `Err` otherwise. Pure so
/// the 404-vs-error decision is unit-testable without a live HTTP round-trip.
fn classify_crate_response(
    status: reqwest::StatusCode,
    url: &str,
    body: String,
) -> Result<Option<String>> {
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        bail!("crates.io API GET {url} returned {status}: {body}");
    }
    Ok(Some(body))
}

/// Pick the research version from a crates.io body, or fall back to the latest
/// published version when no `-research` prerelease exists. Toolchain crates
/// with no `-research` publish yet keep their latest build so the snapshot still
/// assembles; the multi-provider-critical crates (runner/setup/start) carry a
/// `-research` build. The fallback is logged so silent staleness stays visible.
fn research_or_fallback(crate_name: &str, body: &str) -> Result<String> {
    match parse_crates_io_research_version(crate_name, body) {
        Ok(version) => Ok(version),
        Err(_) => {
            let fallback = pick_highest_crates_io_version(crate_name, body, false)?;
            eprintln!(
                "note: `{crate_name}` has no -research publish; the research \
                 toolchain falls back to latest `{fallback}`"
            );
            Ok(fallback)
        }
    }
}

struct ReleaseArtifactVersionResolver<'a> {
    release: &'a str,
}

impl ArtifactVersionResolver for ReleaseArtifactVersionResolver<'_> {
    fn resolve_latest(&self, _package: &str) -> Result<String> {
        Ok(self.release.to_string())
    }
}

struct GhcrArtifactVersionResolver {
    client: reqwest::blocking::Client,
    registry: String,
    namespace: String,
    basic_token: Option<String>,
}

impl GhcrArtifactVersionResolver {
    fn new(raw_token: Option<&str>) -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .build()
                .context("failed to build GHCR HTTP client")?,
            registry: "ghcr.io".to_string(),
            namespace: "greenticai".to_string(),
            basic_token: resolve_registry_token(raw_token)?
                .or_else(|| std::env::var("GHCR_TOKEN").ok())
                .or_else(|| std::env::var("GITHUB_TOKEN").ok()),
        })
    }

    fn bearer_token(&self, repository: &str) -> Result<String> {
        let scope = format!("repository:{repository}:pull");
        let mut request = self
            .client
            .get(format!("https://{}/token", self.registry))
            .query(&[
                ("service", self.registry.as_str()),
                ("scope", scope.as_str()),
            ]);
        if let Some(token) = &self.basic_token {
            request = request.basic_auth(DEFAULT_OAUTH_USER, Some(token));
        }
        let response = request
            .send()
            .with_context(|| format!("failed to request GHCR token for `{repository}`"))?
            .error_for_status()
            .with_context(|| format!("GHCR token request failed for `{repository}`"))?;
        let body: GhcrTokenResponse = response
            .json()
            .with_context(|| format!("failed to parse GHCR token response for `{repository}`"))?;
        Ok(body.token)
    }

    fn tags(&self, repository: &str) -> Result<Vec<String>> {
        let token = self.bearer_token(repository)?;
        let response = self
            .client
            .get(format!(
                "https://{}/v2/{repository}/tags/list",
                self.registry
            ))
            .bearer_auth(token)
            .send()
            .with_context(|| format!("failed to list GHCR tags for `{repository}`"))?
            .error_for_status()
            .with_context(|| format!("GHCR tag list request failed for `{repository}`"))?;
        let body: GhcrTagsResponse = response
            .json()
            .with_context(|| format!("failed to parse GHCR tags for `{repository}`"))?;
        Ok(body.tags)
    }
}

impl ArtifactVersionResolver for GhcrArtifactVersionResolver {
    fn resolve_latest(&self, package: &str) -> Result<String> {
        let repository = format!("{}/{}", self.namespace, package);
        let tags = self.tags(&repository)?;
        select_latest_artifact_tag(&tags)
            .with_context(|| format!("no usable tags found for GHCR package `{repository}`"))
    }
}

#[derive(Deserialize)]
struct GhcrTokenResponse {
    token: String,
}

#[derive(Deserialize)]
struct GhcrTagsResponse {
    #[serde(default)]
    tags: Vec<String>,
}

fn select_latest_artifact_tag(tags: &[String]) -> Result<String> {
    tags.iter()
        .filter_map(|tag| Version::parse(tag).ok().map(|version| (version, tag)))
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, tag)| tag.clone())
        .or_else(|| tags.iter().find(|tag| tag.as_str() == "latest").cloned())
        .context("no semver or latest tags found")
}

impl CrateVersionResolver for CratesIoApiVersionResolver {
    fn resolve_latest(&self, crate_name: &str) -> Result<String> {
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), crate_name);
        let response = self
            .client
            .get(&url)
            .send()
            .with_context(|| format!("failed to GET {url}"))?;
        let status = response.status();
        let body = response
            .text()
            .with_context(|| format!("failed to read body of {url}"))?;
        if !status.is_success() {
            bail!("crates.io API GET {url} returned {status}: {body}");
        }
        parse_crates_io_version(crate_name, &body)
    }

    fn resolve_latest_for_channel(
        &self,
        crate_name: &str,
        channel: ToolchainChannel,
    ) -> Result<String> {
        if channel != ToolchainChannel::Rnd {
            return self.resolve_latest(crate_name);
        }
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), crate_name);
        let body = self.fetch_crate_body(crate_name)?.ok_or_else(|| {
            anyhow!("crates.io API GET {url} returned 404 Not Found (no published `{crate_name}`)")
        })?;
        research_or_fallback(crate_name, &body)
    }

    fn resolve_latest_in_lane(&self, crate_name: &str, lane: (u64, u64)) -> Result<String> {
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), crate_name);
        let body = self.fetch_crate_body(crate_name)?.ok_or_else(|| {
            anyhow!("crates.io API GET {url} returned 404 Not Found (no published `{crate_name}`)")
        })?;
        pick_highest_in_lane(crate_name, &body, lane)
    }

    fn resolve_research_version(&self, crate_name: &str) -> Result<ResearchVersion> {
        // A 404 means the `<name>-rnd` crate is simply not published — the tool
        // ships no research build. Map it to `Absent` (a skip signal) instead of
        // aborting, so `gtc-research install` / manifest assembly survives the
        // ~10 of 13 toolchain crates that have no research line.
        match self.fetch_crate_body(crate_name)? {
            None => Ok(ResearchVersion::Absent),
            Some(body) => research_or_fallback(crate_name, &body).map(ResearchVersion::Pinned),
        }
    }
}

/// Pick the highest non-yanked version from the crates.io `/crates/<name>`
/// response's top-level `versions` array. When `research_only`, restricts to
/// `-research` prereleases (the research lane publishes `X.Y.Z-research`, which
/// `max_stable_version` skips). Otherwise picks the highest semver of ANY
/// channel — the fallback for toolchain crates with no `-research` build, which
/// keeps them at their latest dev build instead of regressing to old stable.
/// The `(major, minor)` lane a release belongs to. greentic versions its
/// toolchain lanes by minor: 1.2.x is dev, 1.3.x is research.
pub(crate) fn lane_of(release: &str) -> Option<(u64, u64)> {
    let mut parts = release.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// Highest non-yanked version of `crate_name` INSIDE `lane`.
///
/// The dev channel must not leave its own minor line. Picking the highest
/// version overall lets an abandoned lane outrank an active one purely on
/// semver ordering — `greentic-setup-dev` stopped publishing 1.3 in July while
/// the dev lane kept shipping 1.2.<run_id>, so every later dev manifest pinned
/// the July build and the channel froze without anyone doing anything wrong.
///
/// An empty lane is an error rather than a fallback: falling back to another
/// lane is the behaviour this function exists to prevent.
fn pick_highest_in_lane(crate_name: &str, body: &str, lane: (u64, u64)) -> Result<String> {
    let payload: serde_json::Value = serde_json::from_str(body)
        .with_context(|| format!("crates.io API for `{crate_name}` returned invalid JSON"))?;
    let versions = payload
        .get("versions")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            anyhow!("crates.io API for `{crate_name}` is missing the `versions` array")
        })?;
    let mut best: Option<Version> = None;
    for entry in versions {
        if entry
            .get("yanked")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(num) = entry.get("num").and_then(|n| n.as_str()) else {
            continue;
        };
        let Ok(parsed) = Version::parse(num) else {
            continue;
        };
        if (parsed.major, parsed.minor) != lane {
            continue;
        }
        if best.as_ref().is_none_or(|current| parsed > *current) {
            best = Some(parsed);
        }
    }
    best.map(|v| v.to_string()).ok_or_else(|| {
        anyhow!(
            "crates.io has no non-yanked `{crate_name}` in the {}.{} lane",
            lane.0,
            lane.1
        )
    })
}

fn pick_highest_crates_io_version(
    crate_name: &str,
    body: &str,
    research_only: bool,
) -> Result<String> {
    let payload: serde_json::Value = serde_json::from_str(body)
        .with_context(|| format!("crates.io API for `{crate_name}` returned invalid JSON"))?;
    let versions = payload
        .get("versions")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            anyhow!("crates.io API for `{crate_name}` is missing the `versions` array")
        })?;
    let mut best: Option<Version> = None;
    for entry in versions {
        if entry
            .get("yanked")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(num) = entry.get("num").and_then(|n| n.as_str()) else {
            continue;
        };
        let Ok(parsed) = Version::parse(num) else {
            continue;
        };
        if research_only && !parsed.pre.as_str().starts_with("research") {
            continue;
        }
        if best.as_ref().is_none_or(|current| parsed > *current) {
            best = Some(parsed);
        }
    }
    best.map(|v| v.to_string()).ok_or_else(|| {
        let what = if research_only {
            "no non-yanked `-research` version"
        } else {
            "no non-yanked versions"
        };
        anyhow!("crates.io API for `{crate_name}` exposes {what}")
    })
}

fn parse_crates_io_research_version(crate_name: &str, body: &str) -> Result<String> {
    pick_highest_crates_io_version(crate_name, body, true)
}

fn parse_crates_io_version(crate_name: &str, body: &str) -> Result<String> {
    let payload: serde_json::Value = serde_json::from_str(body)
        .with_context(|| format!("crates.io API for `{crate_name}` returned invalid JSON"))?;
    let crate_obj = payload.get("crate").ok_or_else(|| {
        anyhow!("crates.io API for `{crate_name}` is missing the top-level `crate` object")
    })?;
    let version = crate_obj
        .get("max_stable_version")
        .and_then(|v| v.as_str())
        .or_else(|| crate_obj.get("newest_version").and_then(|v| v.as_str()))
        .or_else(|| crate_obj.get("max_version").and_then(|v| v.as_str()))
        .ok_or_else(|| {
            anyhow!(
                "crates.io API for `{crate_name}` does not expose max_stable_version, \
                 newest_version, or max_version"
            )
        })?;
    Version::parse(version).with_context(|| {
        format!("crates.io returned an unparseable version `{version}` for `{crate_name}`")
    })?;
    Ok(version.to_string())
}

#[async_trait]
trait ToolchainManifestSource {
    async fn load_manifest(
        &self,
        repo: &str,
        tag: &str,
        token: Option<&str>,
    ) -> Result<Option<ToolchainManifest>>;
}

struct OciToolchainManifestSource;

#[async_trait]
impl ToolchainManifestSource for OciToolchainManifestSource {
    async fn load_manifest(
        &self,
        repo: &str,
        tag: &str,
        token: Option<&str>,
    ) -> Result<Option<ToolchainManifest>> {
        let auth = optional_registry_auth(token)?;
        let client = oci_client();
        let reference = parse_reference(repo, tag)?;
        let image = match client
            .pull(&reference, &auth, vec![TOOLCHAIN_LAYER_MEDIA_TYPE])
            .await
        {
            Ok(image) => image,
            Err(err) if is_missing_manifest_error(&err) || is_unauthorized_error(&err) => {
                return Ok(None);
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to pull {}", toolchain_ref(repo, tag)));
            }
        };
        let Some(layer) = image
            .layers
            .into_iter()
            .find(|layer| layer.media_type == TOOLCHAIN_LAYER_MEDIA_TYPE)
        else {
            return Ok(None);
        };
        let manifest = serde_json::from_slice::<ToolchainManifest>(&layer.data)
            .with_context(|| format!("failed to parse {}", toolchain_ref(repo, tag)))?;
        validate_manifest(&manifest)?;
        Ok(Some(manifest))
    }
}

async fn load_source_manifest(
    repo: &str,
    tag: &str,
    token: Option<&str>,
) -> Result<Option<ToolchainManifest>> {
    OciToolchainManifestSource
        .load_manifest(repo, tag, token)
        .await
}

fn oci_client() -> Client {
    Client::new(ClientConfig {
        protocol: ClientProtocol::Https,
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Updater dispatch — notify the coordinated update-plan workflow
// ---------------------------------------------------------------------------

/// Returns `true` when `version` is a plain `X.Y.Z` (no pre-release, no build
/// metadata) AND `channel` is `"stable"`. The update server must only ever
/// receive stable-lane content; everything else (dev, rnd, run-id versions)
/// is silently skipped.
fn should_notify_updater(version: &str, channel: &str) -> bool {
    if channel != "stable" {
        return false;
    }
    match Version::parse(version) {
        Ok(v) => v.pre.is_empty() && v.build.is_empty(),
        Err(_) => false,
    }
}

/// Resolve a GitHub token from `--token`, then the ambient CI environment. An
/// empty or whitespace-only value counts as absent. Reads of public release
/// metadata work without one; only the dispatch strictly needs it.
pub(crate) fn ambient_github_token(raw_token: Option<&str>) -> Option<String> {
    resolve_registry_token(raw_token)
        .ok()
        .flatten()
        .or_else(|| std::env::var("GHCR_TOKEN").ok())
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .filter(|token| !token.trim().is_empty())
}

/// Fire a `repository_dispatch` to trigger the coordinated update-plan
/// publisher workflow. Failure is a warning, never fatal — the GHCR manifest
/// push already succeeded, and the workflow has a manual `workflow_dispatch`
/// fallback.
fn notify_updater_dispatch(version: &str, channel: &str, raw_token: Option<&str>) {
    if !should_notify_updater(version, channel) {
        eprintln!(
            "Skipping updater dispatch: version `{version}` / channel `{channel}` \
             is not a stable-lane release"
        );
        return;
    }

    let Some(token) = ambient_github_token(raw_token) else {
        eprintln!(
            "Warning: skipping updater dispatch — no token available \
             (pass --token or set GHCR_TOKEN/GITHUB_TOKEN)"
        );
        return;
    };

    let client = match reqwest::blocking::Client::builder()
        .user_agent("greentic-dev-cli")
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: failed to build HTTP client for updater dispatch: {e}");
            return;
        }
    };

    let body = serde_json::json!({
        "event_type": "toolchain-release-published",
        "client_payload": {
            "release": version,
            "channel": channel,
        }
    });

    let url = "https://api.github.com/repos/greenticai/greentic-dev/dispatches";
    match client
        .post(url)
        .header("Accept", "application/vnd.github+json")
        .bearer_auth(&token)
        .json(&body)
        .send()
    {
        Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 204 => {
            eprintln!("Dispatched toolchain-release-published for {version} (channel={channel})");
        }
        Ok(resp) => {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            eprintln!("Warning: updater dispatch returned HTTP {status}: {text}");
        }
        Err(e) => {
            eprintln!("Warning: updater dispatch failed: {e}");
        }
    }
}

fn registry_auth(raw_token: Option<&str>) -> Result<RegistryAuth> {
    let token = resolve_registry_token(raw_token)?
        .or_else(|| std::env::var("GHCR_TOKEN").ok())
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .context("GHCR token is required; pass --token or set GHCR_TOKEN/GITHUB_TOKEN")?;
    if token.trim().is_empty() {
        bail!("GHCR token is empty");
    }
    Ok(RegistryAuth::Basic(DEFAULT_OAUTH_USER.to_string(), token))
}

fn optional_registry_auth(raw_token: Option<&str>) -> Result<RegistryAuth> {
    match registry_auth(raw_token) {
        Ok(auth) => Ok(auth),
        Err(_) if raw_token.is_none() => Ok(RegistryAuth::Anonymous),
        Err(err) => Err(err),
    }
}

fn resolve_registry_token(raw_token: Option<&str>) -> Result<Option<String>> {
    let Some(raw_token) = raw_token else {
        return Ok(None);
    };
    if let Some(var) = raw_token.strip_prefix("env:") {
        let token =
            std::env::var(var).with_context(|| format!("failed to resolve env var {var}"))?;
        if token.trim().is_empty() {
            bail!("env var {var} resolved to an empty token");
        }
        return Ok(Some(token));
    }
    if raw_token.trim().is_empty() {
        bail!("GHCR token is empty");
    }
    Ok(Some(raw_token.to_string()))
}

fn parse_reference(repo: &str, tag: &str) -> Result<Reference> {
    Reference::from_str(&toolchain_ref(repo, tag))
        .with_context(|| format!("invalid OCI reference `{}`", toolchain_ref(repo, tag)))
}

async fn manifest_exists(
    client: &Client,
    reference: &Reference,
    auth: &RegistryAuth,
) -> Result<bool> {
    match client.pull_manifest(reference, auth).await {
        Ok(_) => Ok(true),
        Err(err) if is_missing_manifest_error(&err) => Ok(false),
        Err(err) => Err(err).context("failed to check whether release tag exists"),
    }
}

fn is_missing_manifest_error(
    err: &greentic_distributor_client::oci_client::errors::OciDistributionError,
) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("manifest unknown")
        || msg.contains("name unknown")
        || msg.contains("not found")
        || msg.contains("404")
}

fn is_unauthorized_error(
    err: &greentic_distributor_client::oci_client::errors::OciDistributionError,
) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("not authorized") || msg.contains("unauthorized") || msg.contains("401")
}

async fn push_manifest_layer(
    client: &Client,
    reference: &Reference,
    auth: &RegistryAuth,
    manifest: &ToolchainManifest,
) -> Result<()> {
    let data = serde_json::to_vec_pretty(manifest).context("failed to serialize manifest")?;
    let layer = ImageLayer::new(data, TOOLCHAIN_LAYER_MEDIA_TYPE.to_string(), None);
    let config = Config::new(
        br#"{"toolchain":"gtc"}"#.to_vec(),
        TOOLCHAIN_CONFIG_MEDIA_TYPE.to_string(),
        None,
    );
    client
        .push(reference, &[layer], config, auth, None)
        .await
        .context("failed to push toolchain manifest")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use once_cell::sync::Lazy;
    use std::sync::Mutex;

    static ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    struct FixedResolver;

    impl CrateVersionResolver for FixedResolver {
        fn resolve_latest(&self, crate_name: &str) -> Result<String> {
            Ok(match crate_name {
                "greentic-runner" => "0.5.10",
                _ => "1.2.3",
            }
            .to_string())
        }
    }

    struct FixedArtifactResolver;

    impl ArtifactVersionResolver for FixedArtifactResolver {
        fn resolve_latest(&self, package: &str) -> Result<String> {
            Ok(match package {
                "packs/messaging/messaging-webchat-gui" => "0.4.93",
                "components/component-adaptive-card" => "0.5.8",
                _ => "0.1.0",
            }
            .to_string())
        }
    }

    #[test]
    fn parses_crates_io_max_stable_version() {
        let body = r#"{"crate":{"id":"greentic-operator-dev","max_stable_version":"0.5.123"}}"#;
        let version = parse_crates_io_version("greentic-operator-dev", body).unwrap();
        assert_eq!(version, "0.5.123");
    }

    #[test]
    fn research_resolver_picks_highest_non_yanked_research_prerelease() {
        // The research lane must ignore the stable `max_stable_version` (0.5.48)
        // and pick the highest non-yanked `-research` prerelease.
        let body = r#"{"crate":{"id":"greentic-runner","max_stable_version":"0.5.48"},
            "versions":[
                {"num":"0.5.48","yanked":false},
                {"num":"1.2.0-research.0","yanked":false},
                {"num":"1.2.0-research.1","yanked":false},
                {"num":"1.2.0-research.2","yanked":true}
            ]}"#;
        let version = parse_crates_io_research_version("greentic-runner", body).unwrap();
        assert_eq!(version, "1.2.0-research.1");
    }

    #[test]
    fn research_resolver_errors_when_no_research_version() {
        let body = r#"{"crate":{"id":"greentic-setup"},
            "versions":[{"num":"1.2.0-dev.123","yanked":false},{"num":"0.5.25","yanked":false}]}"#;
        let err = parse_crates_io_research_version("greentic-setup", body).unwrap_err();
        assert!(err.to_string().contains("no non-yanked `-research`"));
    }

    /// Mirrors the real fleet: only start/runner/setup ship `-research` crates,
    /// so any `operator` crate resolves to `Absent` (a skip), everything else to
    /// a pinned research version.
    struct ResearchSkipResolver;

    impl CrateVersionResolver for ResearchSkipResolver {
        fn resolve_latest(&self, _crate_name: &str) -> Result<String> {
            Ok("1.2.0-research.4".to_string())
        }

        fn resolve_research_version(&self, crate_name: &str) -> Result<ResearchVersion> {
            if crate_name.contains("operator") {
                Ok(ResearchVersion::Absent)
            } else {
                Ok(ResearchVersion::Pinned("1.2.0-research.4".to_string()))
            }
        }
    }

    #[test]
    fn classify_crate_response_maps_404_to_absent() {
        let outcome =
            classify_crate_response(reqwest::StatusCode::NOT_FOUND, "url", "missing".to_string())
                .unwrap();
        assert!(outcome.is_none(), "404 must classify as absent (None)");
    }

    #[test]
    fn classify_crate_response_returns_body_on_success() {
        let outcome =
            classify_crate_response(reqwest::StatusCode::OK, "url", "payload".to_string()).unwrap();
        assert_eq!(outcome.as_deref(), Some("payload"));
    }

    #[test]
    fn classify_crate_response_errors_on_other_status() {
        let err = classify_crate_response(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "url",
            "boom".to_string(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("500"));
    }

    #[test]
    fn snapshot_manifest_skips_crates_without_research_build() {
        // The absent `operator` crate must be omitted, not abort the whole
        // research manifest — the regression behind .github#212's install hang.
        let manifest = snapshot_manifest(
            "1.2.0-research.4",
            ToolchainChannel::Rnd,
            &ResearchSkipResolver,
            None,
        )
        .unwrap();
        assert!(
            !manifest.packages.is_empty(),
            "research-built tools must remain in the manifest"
        );
        assert!(
            manifest.packages.len() < GREENTIC_TOOLCHAIN_PACKAGES.len(),
            "at least one tool without a research build must be skipped"
        );
        assert!(
            manifest
                .packages
                .iter()
                .all(|package| !package.crate_name.contains("operator")),
            "the absent `operator` crate must be omitted"
        );
        assert!(
            manifest
                .packages
                .iter()
                .all(|package| package.version == "1.2.0-research.4"),
            "remaining research tools pin their resolved -research version"
        );
    }

    #[test]
    fn parses_crates_io_falls_back_to_newest_version() {
        let body = r#"{"crate":{"id":"greentic-flow-dev","newest_version":"0.6.7"}}"#;
        let version = parse_crates_io_version("greentic-flow-dev", body).unwrap();
        assert_eq!(version, "0.6.7");
    }

    #[test]
    fn parses_crates_io_falls_back_to_max_version() {
        let body = r#"{"crate":{"id":"greentic-runner-dev","max_version":"0.4.99"}}"#;
        let version = parse_crates_io_version("greentic-runner-dev", body).unwrap();
        assert_eq!(version, "0.4.99");
    }

    #[test]
    fn rejects_crates_io_payload_without_versions() {
        let body = r#"{"crate":{"id":"greentic-dev"}}"#;
        let err = parse_crates_io_version("greentic-dev", body).unwrap_err();
        assert!(
            err.to_string()
                .contains("does not expose max_stable_version")
        );
    }

    #[test]
    fn rejects_crates_io_payload_with_unparseable_version() {
        let body = r#"{"crate":{"max_stable_version":"not-a-version"}}"#;
        let err = parse_crates_io_version("greentic-dev", body).unwrap_err();
        assert!(err.to_string().contains("unparseable version"));
    }

    #[test]
    fn rejects_crates_io_payload_without_crate_object() {
        let body = r#"{"errors":[{"detail":"not found"}]}"#;
        let err = parse_crates_io_version("greentic-dev", body).unwrap_err();
        assert!(
            err.to_string()
                .contains("missing the top-level `crate` object")
        );
    }

    #[test]
    fn selects_latest_semver_tag() {
        let tags = vec![
            "latest".to_string(),
            "0.4.93".to_string(),
            "0.4.9".to_string(),
            "1.0.0-beta.1".to_string(),
            "1.0.0".to_string(),
        ];

        assert_eq!(select_latest_artifact_tag(&tags).unwrap(), "1.0.0");
    }

    #[test]
    fn selects_latest_tag_when_no_semver_tags_exist() {
        let tags = vec!["latest".to_string()];

        assert_eq!(select_latest_artifact_tag(&tags).unwrap(), "latest");
    }

    #[test]
    fn generates_manifest_from_catalogue() {
        let manifest = generate_manifest("1.0.5", "latest", None, &FixedResolver, None).unwrap();
        assert_eq!(manifest.schema, TOOLCHAIN_MANIFEST_SCHEMA);
        assert_eq!(manifest.toolchain, TOOLCHAIN_NAME);
        assert_eq!(manifest.version, "1.0.5");
        assert_eq!(manifest.channel.as_deref(), Some("latest"));
        assert!(
            manifest
                .packages
                .iter()
                .any(|package| package.crate_name == "greentic-bundle"
                    && package.bins == ["greentic-bundle"])
        );
        assert!(
            manifest
                .packages
                .iter()
                .any(|package| package.crate_name == "greentic-runner"
                    && package.bins == ["greentic-runner"])
        );
        assert_eq!(manifest.extension_packs.as_ref().unwrap().len(), 81);
        assert_eq!(manifest.components.as_ref().unwrap().len(), 10);
        assert!(
            manifest
                .extension_packs
                .as_ref()
                .unwrap()
                .iter()
                .all(|item| item.version == "1.0.5")
        );
        assert!(
            manifest
                .components
                .as_ref()
                .unwrap()
                .iter()
                .all(|item| item.version == "1.0.5")
        );
    }

    #[test]
    fn generated_manifest_can_use_artifact_resolver_versions() {
        let manifest = generate_manifest_with_artifact_resolver(
            "1.0.17",
            "stable",
            None,
            &FixedResolver,
            &FixedArtifactResolver,
            None,
        )
        .unwrap();

        assert!(
            manifest
                .extension_packs
                .as_ref()
                .unwrap()
                .iter()
                .any(|item| item.id == "packs/messaging/messaging-webchat-gui"
                    && item.version == "0.4.93")
        );
        assert!(
            manifest
                .components
                .as_ref()
                .unwrap()
                .iter()
                .any(|item| item.id == "components/component-adaptive-card"
                    && item.version == "0.5.8")
        );
    }

    #[test]
    fn source_manifest_can_pin_package_versions() {
        let source = ToolchainManifest {
            schema: TOOLCHAIN_MANIFEST_SCHEMA.to_string(),
            toolchain: TOOLCHAIN_NAME.to_string(),
            version: "latest".to_string(),
            channel: Some("latest".to_string()),
            created_at: None,
            packages: vec![ToolchainPackage {
                crate_name: "greentic-dev".to_string(),
                bins: vec!["greentic-dev".to_string()],
                version: "0.5.9".to_string(),
                artifacts: None,
            }],
            extension_packs: None,
            components: None,
            gtc: None,
        };
        let manifest =
            generate_manifest("1.0.5", "latest", Some(&source), &FixedResolver, None).unwrap();
        let greentic_dev = manifest
            .packages
            .iter()
            .find(|package| package.crate_name == "greentic-dev")
            .unwrap();
        assert_eq!(greentic_dev.version, "0.5.9");
    }

    #[test]
    fn from_argument_controls_generated_channel_over_source_manifest() {
        let source = ToolchainManifest {
            schema: TOOLCHAIN_MANIFEST_SCHEMA.to_string(),
            toolchain: TOOLCHAIN_NAME.to_string(),
            version: "latest".to_string(),
            channel: Some("stable".to_string()),
            created_at: None,
            packages: Vec::new(),
            extension_packs: None,
            components: None,
            gtc: None,
        };
        let manifest =
            generate_manifest("1.0.16", "dev", Some(&source), &FixedResolver, None).unwrap();
        assert_eq!(manifest.channel.as_deref(), Some("dev"));
        assert_eq!(manifest_file_name(&manifest), "gtc-dev-1.0.16.json");
    }

    #[test]
    fn generate_from_dev_uses_dev_crate_and_binary_names() {
        let manifest = generate_manifest("1.0.16", "dev", None, &FixedResolver, None).unwrap();
        assert!(
            manifest
                .packages
                .iter()
                .flat_map(|package| package.bins.iter())
                .all(|bin| bin.ends_with("-dev"))
        );
        assert!(
            manifest
                .packages
                .iter()
                .all(|package| package.crate_name.ends_with("-dev")),
            "dev manifest must pin -dev crate names so binstall resolves the dev mirror"
        );
        assert!(manifest.packages.iter().any(|package| {
            package.crate_name == "greentic-flow-dev" && package.bins == ["greentic-flow-dev"]
        }));
        assert!(manifest.packages.iter().any(|package| {
            package.crate_name == "greentic-component-dev"
                && package.bins == ["greentic-component-dev"]
        }));
        assert!(manifest.packages.iter().any(|package| {
            package.crate_name == "greentic-dev-dev" && package.bins == ["greentic-dev-dev"]
        }));
    }

    #[test]
    fn snapshot_manifest_dev_channel_resolves_dev_aliases() {
        let manifest =
            snapshot_manifest("1.1.5", ToolchainChannel::Development, &FixedResolver, None)
                .unwrap();
        assert_eq!(manifest.version, "1.1.5");
        assert_eq!(manifest.channel.as_deref(), Some("dev"));
        for package in &manifest.packages {
            assert!(
                package.crate_name.ends_with("-dev"),
                "dev snapshot must pin -dev crate names; got {}",
                package.crate_name
            );
            assert!(
                package.bins.iter().all(|bin| bin.ends_with("-dev")),
                "dev snapshot must pin -dev bin names; got {:?}",
                package.bins
            );
            assert_ne!(
                package.version, "latest",
                "snapshot must always resolve concrete versions"
            );
        }
        assert!(
            manifest
                .packages
                .iter()
                .any(|package| package.crate_name == "greentic-operator-dev")
        );
    }

    #[test]
    fn snapshot_manifest_stable_channel_keeps_plain_names() {
        let manifest =
            snapshot_manifest("1.0.20", ToolchainChannel::Stable, &FixedResolver, None).unwrap();
        assert_eq!(manifest.channel.as_deref(), Some("stable"));
        // The stable channel must NOT apply the `-dev` suffix transform.
        // Cross-check against the catalogue: every stable package must match
        // a catalogue entry by exact name (no transform applied).
        let catalogue_names: std::collections::BTreeSet<_> = GREENTIC_TOOLCHAIN_PACKAGES
            .iter()
            .map(|spec| spec.crate_name)
            .collect();
        for package in &manifest.packages {
            assert!(
                catalogue_names.contains(package.crate_name.as_str()),
                "stable snapshot crate `{}` was transformed; expected a verbatim catalogue entry",
                package.crate_name
            );
        }
    }

    #[test]
    fn snapshot_manifest_resolves_via_resolver() {
        let manifest =
            snapshot_manifest("1.1.6", ToolchainChannel::Development, &FixedResolver, None)
                .unwrap();
        // FixedResolver returns 1.2.3 for everything except `greentic-runner`.
        // The dev channel queries `greentic-runner-dev`, not `greentic-runner`,
        // so the special case in FixedResolver does not apply and every
        // package should land on the default 1.2.3 — proving the resolver was
        // hit (rather than versions copied from somewhere).
        for package in &manifest.packages {
            assert_eq!(
                package.version, "1.2.3",
                "resolver must be hit for {}",
                package.crate_name
            );
        }
    }

    #[test]
    fn parses_dev_channel_argument() {
        assert_eq!(parse_channel("dev").unwrap(), ToolchainChannel::Development);
        assert_eq!(
            parse_channel("development").unwrap(),
            ToolchainChannel::Development
        );
        assert_eq!(parse_channel("stable").unwrap(), ToolchainChannel::Stable);
        assert!(parse_channel("rc").is_err());
    }

    /// The dev channel must stay inside its own minor line.
    ///
    /// greentic uses 1.2.x for the dev lane and 1.3.x for research. Picking the
    /// highest version overall makes an ABANDONED research build outrank an
    /// active dev one: `greentic-setup-dev` published 1.3.29488015798 in July
    /// and nothing since, while the dev lane kept shipping 1.2.<run_id>. Every
    /// dev manifest generated after that pinned the July build, which is how
    /// the dev channel silently froze.
    #[test]
    fn the_dev_lane_ignores_a_higher_research_minor() {
        let body = r#"{"versions":[
            {"num":"1.2.32329835532","yanked":false},
            {"num":"1.2.32374877786","yanked":false},
            {"num":"1.3.29293243074","yanked":false},
            {"num":"1.3.29488015798","yanked":false}
        ]}"#;

        assert_eq!(
            pick_highest_in_lane("greentic-setup-dev", body, (1, 2)).unwrap(),
            "1.2.32374877786",
            "the newest 1.2 build must win over any 1.3"
        );
        assert_eq!(
            pick_highest_in_lane("greentic-setup-dev", body, (1, 3)).unwrap(),
            "1.3.29488015798",
            "asking for the 1.3 lane still resolves inside 1.3"
        );
    }

    /// A yanked build must never be pinned, lane or not.
    #[test]
    fn a_yanked_build_is_not_pinned_in_lane() {
        let body = r#"{"versions":[
            {"num":"1.2.100","yanked":false},
            {"num":"1.2.200","yanked":true}
        ]}"#;
        assert_eq!(pick_highest_in_lane("c", body, (1, 2)).unwrap(), "1.2.100");
    }

    /// A crate with nothing in the lane is an error the caller can report,
    /// not a silent fall back to another lane — falling back is the bug.
    #[test]
    fn an_empty_lane_is_an_error_not_a_fallback() {
        let body = r#"{"versions":[{"num":"1.3.5","yanked":false}]}"#;
        let err = pick_highest_in_lane("c", body, (1, 2)).unwrap_err();
        assert!(
            err.to_string().contains("1.2"),
            "the error must name the lane it searched; got {err}"
        );
    }

    /// `--release 1.2.1` means the 1.2 lane.
    #[test]
    fn the_lane_comes_from_the_release_being_generated() {
        assert_eq!(lane_of("1.2.1"), Some((1, 2)));
        assert_eq!(lane_of("1.2.32374413367"), Some((1, 2)));
        assert_eq!(lane_of("nonsense"), None);
    }

    #[test]
    fn detects_concrete_pins_for_publish_deprecation_warning() {
        let with_pins = ToolchainManifest {
            schema: TOOLCHAIN_MANIFEST_SCHEMA.to_string(),
            toolchain: TOOLCHAIN_NAME.to_string(),
            version: "0.0.1".to_string(),
            channel: Some("dev".to_string()),
            created_at: None,
            packages: vec![ToolchainPackage {
                crate_name: "greentic-operator-dev".to_string(),
                bins: vec!["greentic-operator-dev".to_string()],
                version: "0.5.123".to_string(),
                artifacts: None,
            }],
            extension_packs: None,
            components: None,
            gtc: None,
        };
        assert!(source_manifest_has_concrete_pins(&with_pins));

        let only_latest = ToolchainManifest {
            packages: vec![ToolchainPackage {
                crate_name: "greentic-operator".to_string(),
                bins: vec!["greentic-operator".to_string()],
                version: "latest".to_string(),
                artifacts: None,
            }],
            ..with_pins
        };
        assert!(!source_manifest_has_concrete_pins(&only_latest));
    }

    #[test]
    fn generate_from_rnd_uses_rnd_binary_names() {
        let manifest = generate_manifest("1.2.0", "rnd", None, &FixedResolver, None).unwrap();
        assert_eq!(manifest.channel.as_deref(), Some("rnd"));
        assert!(
            manifest
                .packages
                .iter()
                .flat_map(|package| package.bins.iter())
                .all(|bin| bin.ends_with("-rnd"))
        );
        assert!(manifest.packages.iter().any(|package| {
            package.crate_name == "greentic-flow" && package.bins == ["greentic-flow-rnd"]
        }));
    }

    #[test]
    fn bootstrap_source_manifest_uses_source_tag_identity() {
        let manifest = bootstrap_source_manifest("latest", &FixedResolver, None).unwrap();
        assert_eq!(manifest.version, "latest");
        assert_eq!(manifest.channel.as_deref(), Some("latest"));
        assert_eq!(manifest.schema, TOOLCHAIN_MANIFEST_SCHEMA);
        assert_eq!(manifest.toolchain, TOOLCHAIN_NAME);
        assert!(
            manifest
                .packages
                .iter()
                .all(|package| package.version != "latest")
        );
    }

    #[test]
    fn validates_schema_and_toolchain() {
        let mut manifest =
            generate_manifest("1.0.5", "latest", None, &FixedResolver, None).unwrap();
        assert!(validate_manifest(&manifest).is_ok());
        manifest.schema = "wrong".to_string();
        assert!(validate_manifest(&manifest).is_err());
        manifest.schema = TOOLCHAIN_MANIFEST_SCHEMA.to_string();
        manifest.toolchain = "other".to_string();
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn resolves_inline_registry_token() {
        assert_eq!(
            resolve_registry_token(Some("secret-token"))
                .unwrap()
                .as_deref(),
            Some("secret-token")
        );
    }

    #[test]
    fn resolves_registry_token_from_environment_reference() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = std::env::var("RELEASE_CMD_TEST_TOKEN").ok();
        unsafe { std::env::set_var("RELEASE_CMD_TEST_TOKEN", "env-secret") };

        let resolved = resolve_registry_token(Some("env:RELEASE_CMD_TEST_TOKEN")).unwrap();
        assert_eq!(resolved.as_deref(), Some("env-secret"));

        match previous {
            Some(value) => unsafe { std::env::set_var("RELEASE_CMD_TEST_TOKEN", value) },
            None => unsafe { std::env::remove_var("RELEASE_CMD_TEST_TOKEN") },
        }
    }

    #[test]
    fn rejects_empty_registry_token_from_environment_reference() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = std::env::var("RELEASE_CMD_TEST_TOKEN").ok();
        unsafe { std::env::set_var("RELEASE_CMD_TEST_TOKEN", "   ") };

        let err = resolve_registry_token(Some("env:RELEASE_CMD_TEST_TOKEN")).unwrap_err();
        assert!(err.to_string().contains("resolved to an empty token"));

        match previous {
            Some(value) => unsafe { std::env::set_var("RELEASE_CMD_TEST_TOKEN", value) },
            None => unsafe { std::env::remove_var("RELEASE_CMD_TEST_TOKEN") },
        }
    }

    #[test]
    fn registry_auth_uses_environment_fallbacks() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous_ghcr = std::env::var("GHCR_TOKEN").ok();
        let previous_github = std::env::var("GITHUB_TOKEN").ok();
        unsafe { std::env::set_var("GHCR_TOKEN", "ghcr-secret") };
        unsafe { std::env::remove_var("GITHUB_TOKEN") };

        let auth = registry_auth(None).unwrap();
        match auth {
            RegistryAuth::Basic(user, token) => {
                assert_eq!(user, DEFAULT_OAUTH_USER);
                assert_eq!(token, "ghcr-secret");
            }
            _ => panic!("expected basic auth"),
        }

        match previous_ghcr {
            Some(value) => unsafe { std::env::set_var("GHCR_TOKEN", value) },
            None => unsafe { std::env::remove_var("GHCR_TOKEN") },
        }
        match previous_github {
            Some(value) => unsafe { std::env::set_var("GITHUB_TOKEN", value) },
            None => unsafe { std::env::remove_var("GITHUB_TOKEN") },
        }
    }

    #[test]
    fn optional_registry_auth_allows_missing_implicit_token() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous_ghcr = std::env::var("GHCR_TOKEN").ok();
        let previous_github = std::env::var("GITHUB_TOKEN").ok();
        unsafe { std::env::remove_var("GHCR_TOKEN") };
        unsafe { std::env::remove_var("GITHUB_TOKEN") };

        let auth = optional_registry_auth(None).unwrap();
        assert!(matches!(auth, RegistryAuth::Anonymous));

        match previous_ghcr {
            Some(value) => unsafe { std::env::set_var("GHCR_TOKEN", value) },
            None => unsafe { std::env::remove_var("GHCR_TOKEN") },
        }
        match previous_github {
            Some(value) => unsafe { std::env::set_var("GITHUB_TOKEN", value) },
            None => unsafe { std::env::remove_var("GITHUB_TOKEN") },
        }
    }

    #[test]
    fn release_view_tag_prefers_release_or_tag() {
        let args = ReleaseViewArgs {
            release: Some("1.0.5".to_string()),
            tag: None,
            repo: "ghcr.io/greenticai/greentic-versions/gtc".to_string(),
            token: None,
        };
        assert_eq!(release_view_tag(&args).unwrap(), "1.0.5");

        let args = ReleaseViewArgs {
            release: None,
            tag: Some("stable".to_string()),
            repo: "ghcr.io/greenticai/greentic-versions/gtc".to_string(),
            token: None,
        };
        assert_eq!(release_view_tag(&args).unwrap(), "stable");
    }

    #[test]
    fn release_view_tag_rejects_invalid_argument_combinations() {
        let err = release_view_tag(&ReleaseViewArgs {
            release: Some("1.0.5".to_string()),
            tag: Some("stable".to_string()),
            repo: "ghcr.io/greenticai/greentic-versions/gtc".to_string(),
            token: None,
        })
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("pass exactly one of --release or --tag")
        );

        let err = release_view_tag(&ReleaseViewArgs {
            release: None,
            tag: None,
            repo: "ghcr.io/greenticai/greentic-versions/gtc".to_string(),
            token: None,
        })
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("pass exactly one of --release or --tag")
        );
    }

    #[test]
    fn publish_manifest_input_uses_local_manifest_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gtc-1.0.12.json");
        let manifest = generate_manifest("1.0.12", "latest", None, &FixedResolver, None).unwrap();
        fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        let args = ReleasePublishArgs {
            release: None,
            from: None,
            tag: Some("stable".to_string()),
            manifest: Some(path.clone()),
            repo: "ghcr.io/greenticai/greentic-versions/gtc".to_string(),
            token: None,
            out: dir.path().to_path_buf(),
            dry_run: true,
            force: true,
            no_notify_updater: true,
        };
        let (release, loaded, source_path) = publish_manifest_input(&args).unwrap();
        assert_eq!(release, "1.0.12");
        assert_eq!(loaded, manifest);
        assert_eq!(
            source_path,
            Some(PublishManifestSource::Local(path.clone()))
        );
    }

    #[test]
    fn publish_manifest_input_allows_release_override_for_local_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gtc-1.0.13.json");
        let manifest = generate_manifest("1.0.12", "latest", None, &FixedResolver, None).unwrap();
        fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        let args = ReleasePublishArgs {
            release: Some("1.0.13".to_string()),
            from: None,
            tag: Some("stable".to_string()),
            manifest: Some(path.clone()),
            repo: "ghcr.io/greenticai/greentic-versions/gtc".to_string(),
            token: None,
            out: dir.path().to_path_buf(),
            dry_run: true,
            force: true,
            no_notify_updater: true,
        };
        let (release, loaded, source_path) = publish_manifest_input(&args).unwrap();
        assert_eq!(release, "1.0.13");
        assert_eq!(loaded.version, "1.0.13");
        assert_eq!(
            source_path,
            Some(PublishManifestSource::Local(path.clone()))
        );
    }

    #[test]
    fn manifest_file_name_omits_stable_channel() {
        let manifest = ToolchainManifest {
            schema: TOOLCHAIN_MANIFEST_SCHEMA.to_string(),
            toolchain: TOOLCHAIN_NAME.to_string(),
            version: "1.0.12".to_string(),
            channel: Some("stable".to_string()),
            created_at: None,
            packages: Vec::new(),
            extension_packs: None,
            components: None,
            gtc: None,
        };
        assert_eq!(manifest_file_name(&manifest), "gtc-1.0.12.json");
    }

    #[test]
    fn parses_manifest_without_extension_sections() {
        let manifest: ToolchainManifest = serde_json::from_str(
            r#"{
              "schema": "greentic.toolchain-manifest.v1",
              "toolchain": "gtc",
              "version": "1.0.16",
              "channel": "stable",
              "packages": []
            }"#,
        )
        .unwrap();

        assert_eq!(manifest.extension_packs, None);
        assert_eq!(manifest.components, None);
    }

    #[test]
    fn generated_manifest_includes_catalogue_extension_sections() {
        let manifest = generate_manifest("1.0.16", "stable", None, &FixedResolver, None).unwrap();
        let json = serde_json::to_value(&manifest).unwrap();

        assert!(json.get("extension_packs").is_some());
        assert!(json.get("components").is_some());
        assert!(
            manifest
                .extension_packs
                .as_ref()
                .unwrap()
                .iter()
                .any(|item| item.id == "packs/messaging/messaging-webchat-gui"
                    && item.version == "1.0.16")
        );
        assert!(
            manifest
                .extension_packs
                .as_ref()
                .unwrap()
                .iter()
                .any(|item| item.id == "packs/deployer/greentic.deploy.aws"
                    && item.version == "1.0.16")
        );
        assert!(
            manifest
                .components
                .as_ref()
                .unwrap()
                .iter()
                .any(|item| item.id == "component/component-llm-openai"
                    && item.version == "1.0.16")
        );
    }

    #[test]
    fn generated_manifest_preserves_source_versions_for_tracked_extension_sections() {
        let source = ToolchainManifest {
            schema: TOOLCHAIN_MANIFEST_SCHEMA.to_string(),
            toolchain: TOOLCHAIN_NAME.to_string(),
            version: "dev".to_string(),
            channel: Some("dev".to_string()),
            created_at: None,
            packages: Vec::new(),
            extension_packs: Some(vec![ExtensionPackRef {
                id: "packs/messaging/messaging-webchat-gui".to_string(),
                version: "0.5.4".to_string(),
            }]),
            components: Some(vec![ComponentRef {
                id: "components/component-adaptive-card".to_string(),
                version: "0.5.8".to_string(),
            }]),
            gtc: None,
        };

        let manifest =
            generate_manifest("1.0.16", "stable", Some(&source), &FixedResolver, None).unwrap();

        assert!(
            manifest
                .extension_packs
                .as_ref()
                .unwrap()
                .iter()
                .any(|item| item.id == "packs/messaging/messaging-webchat-gui"
                    && item.version == "0.5.4")
        );
        assert!(
            manifest
                .components
                .as_ref()
                .unwrap()
                .iter()
                .any(|item| item.id == "components/component-adaptive-card"
                    && item.version == "0.5.8")
        );
    }

    #[test]
    fn manifest_file_name_includes_non_stable_channel() {
        let mut manifest = generate_manifest("1.0.12", "dev", None, &FixedResolver, None).unwrap();
        assert_eq!(manifest_file_name(&manifest), "gtc-dev-1.0.12.json");

        manifest.channel = Some("customer-a".to_string());
        assert_eq!(manifest_file_name(&manifest), "gtc-customer-a-1.0.12.json");
    }

    #[test]
    fn manifest_helpers_only_apply_dev_suffix_for_dev_channel() {
        assert_eq!(
            manifest_bins_for_source("latest", &["greentic-dev", "greentic-runner"]),
            vec!["greentic-dev".to_string(), "greentic-runner".to_string()]
        );
        assert_eq!(
            manifest_bins_for_source("dev", &["greentic-dev"]),
            vec!["greentic-dev-dev".to_string()]
        );
        assert_eq!(
            manifest_crate_name_for_source("latest", "greentic-runner"),
            "greentic-runner"
        );
        assert_eq!(
            manifest_crate_name_for_source("dev", "greentic-runner"),
            "greentic-runner-dev"
        );
    }

    #[test]
    fn source_version_map_handles_missing_and_present_sources() {
        assert!(source_version_map(None).is_empty());

        let source = ToolchainManifest {
            schema: TOOLCHAIN_MANIFEST_SCHEMA.to_string(),
            toolchain: TOOLCHAIN_NAME.to_string(),
            version: "latest".to_string(),
            channel: Some("latest".to_string()),
            created_at: None,
            packages: vec![ToolchainPackage {
                crate_name: "greentic-dev".to_string(),
                bins: vec!["greentic-dev".to_string()],
                version: "0.6.0".to_string(),
                artifacts: None,
            }],
            extension_packs: None,
            components: None,
            gtc: None,
        };

        let versions = source_version_map(Some(&source));
        assert_eq!(
            versions.get("greentic-dev").map(String::as_str),
            Some("0.6.0")
        );
    }

    #[test]
    fn write_manifest_persists_json_to_expected_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = generate_manifest("1.0.12", "dev", None, &FixedResolver, None).unwrap();

        let path = write_manifest(dir.path(), &manifest).unwrap();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("gtc-dev-1.0.12.json")
        );

        let roundtrip = read_manifest_file(&path).unwrap();
        assert_eq!(roundtrip, manifest);
    }

    #[test]
    fn latest_manifest_uses_latest_dev_bins() {
        let manifest = latest_manifest(None);
        assert_eq!(manifest.version, "latest");
        assert_eq!(manifest.channel.as_deref(), Some("latest"));
        assert_eq!(manifest.schema, TOOLCHAIN_MANIFEST_SCHEMA);
        assert_eq!(manifest.toolchain, TOOLCHAIN_NAME);
        assert!(!manifest.packages.is_empty());
        assert!(
            manifest
                .packages
                .iter()
                .all(|package| package.version == "latest")
        );
        assert!(
            manifest
                .packages
                .iter()
                .flat_map(|package| package.bins.iter())
                .all(|bin| bin.ends_with("-dev"))
        );
        assert!(
            manifest
                .packages
                .iter()
                .all(|package| package.crate_name.ends_with("-dev")),
            "latest-channel manifest mirrors dev binaries, so crate names must be -dev too"
        );
        assert!(
            manifest
                .packages
                .iter()
                .any(|package| { package.crate_name == "gtc-dev" && package.bins == ["gtc-dev"] })
        );
        assert!(manifest.packages.iter().any(|package| {
            package.crate_name == "greentic-dev-dev" && package.bins == ["greentic-dev-dev"]
        }));
        assert!(
            manifest
                .extension_packs
                .as_ref()
                .unwrap()
                .iter()
                .all(|item| item.version == "latest")
        );
        assert!(
            manifest
                .components
                .as_ref()
                .unwrap()
                .iter()
                .all(|item| item.version == "latest")
        );
    }

    #[test]
    fn publish_dry_run_with_local_manifest_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gtc-1.0.12.json");
        let manifest = generate_manifest("1.0.12", "latest", None, &FixedResolver, None).unwrap();
        fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

        // `--tag stable` puts this dry run inside the stable gate, so it needs a
        // checker; a live one would reach for github.com from a unit test.
        let checker = StubReleaseChecker::complete_for(&manifest);
        publish_with_checker(
            ReleasePublishArgs {
                release: None,
                from: None,
                tag: Some("stable".to_string()),
                manifest: Some(path),
                repo: "ghcr.io/greenticai/greentic-versions/gtc".to_string(),
                token: None,
                out: dir.path().to_path_buf(),
                dry_run: true,
                force: false,
                no_notify_updater: true,
            },
            &checker,
        )
        .unwrap();
    }

    // -----------------------------------------------------------------------
    // verify_manifest_releases — stable-lane release gate
    // -----------------------------------------------------------------------

    /// Maps `"<crate>@<tag>"` to that release's asset names. An absent key means
    /// the release does not exist.
    struct StubReleaseChecker(BTreeMap<String, Vec<String>>);

    impl ReleaseAssetChecker for StubReleaseChecker {
        fn release_assets(&self, repo: &str, tag: &str) -> Result<Option<Vec<String>>> {
            Ok(self.0.get(&format!("{repo}@{tag}")).cloned())
        }
    }

    /// A checker that answers `release_artifacts` — the stub above deliberately
    /// does not, so it also proves the trait default keeps working.
    struct StubArtifactChecker(Vec<ReleaseArtifact>);

    impl ReleaseAssetChecker for StubArtifactChecker {
        fn release_assets(&self, _repo: &str, _tag: &str) -> Result<Option<Vec<String>>> {
            Ok(Some(self.0.iter().map(|a| a.name.clone()).collect()))
        }

        fn release_artifacts(&self, repo: &str, tag: &str) -> Result<Option<Vec<ReleaseArtifact>>> {
            assert_eq!(repo, GTC_RELEASE_REPO, "gtc is looked up in its own repo");
            assert_eq!(tag, "v1.2.3");
            Ok(Some(
                self.0
                    .iter()
                    .map(|a| ReleaseArtifact {
                        name: a.name.clone(),
                        url: a.url.clone(),
                        sha256: a.sha256.clone(),
                    })
                    .collect(),
            ))
        }
    }

    fn artifact(name: &str, digest: Option<&str>) -> ReleaseArtifact {
        ReleaseArtifact {
            name: name.to_string(),
            url: Some(format!(
                "https://github.com/greenticai/greentic/releases/download/v1.2.3/{name}"
            )),
            sha256: digest.map(str::to_string),
        }
    }

    #[test]
    fn gtc_artifacts_are_taken_from_the_release_not_rebuilt() {
        // Dev-lane naming: the very shape the consumer could not reconstruct.
        let checker = StubArtifactChecker(vec![
            artifact(
                "gtc-dev-v1.2.3-x86_64-unknown-linux-gnu.tgz",
                Some(&format!("sha256:{}", "a".repeat(64))),
            ),
            artifact(
                "gtc-dev-v1.2.3-x86_64-unknown-linux-gnu.tgz.sha256",
                Some(&format!("sha256:{}", "b".repeat(64))),
            ),
        ]);

        let named = gtc_artifacts_for("1.2.3", &checker)
            .expect("lookup")
            .expect("some");
        assert_eq!(named.len(), 1, "the .sha256 sidecar is not an artifact");
        assert_eq!(named[0].target, "x86_64-unknown-linux-gnu");
        assert!(
            named[0]
                .url
                .ends_with("gtc-dev-v1.2.3-x86_64-unknown-linux-gnu.tgz")
        );
        // Stored bare, matching the checksums-manifest shape the other path uses.
        assert_eq!(named[0].sha256, "a".repeat(64));
    }

    #[test]
    fn an_asset_without_a_digest_is_skipped_rather_than_published_unverifiable() {
        let checker = StubArtifactChecker(vec![artifact(
            "gtc-dev-v1.2.3-aarch64-apple-darwin.tgz",
            None,
        )]);
        assert!(
            gtc_artifacts_for("1.2.3", &checker)
                .expect("lookup")
                .is_none()
        );
    }

    #[test]
    fn a_checker_that_reports_no_artifacts_leaves_the_field_absent() {
        // The trait default. Absent means the consumer reconstructs, exactly as
        // every manifest published before this field existed.
        let manifest = latest_manifest(None);
        let stub = StubReleaseChecker::complete_for(&manifest);
        assert!(gtc_artifacts_for("1.2.3", &stub).expect("lookup").is_none());
    }

    impl StubReleaseChecker {
        /// Every package in `manifest` present with a complete asset set.
        fn complete_for(manifest: &ToolchainManifest) -> Self {
            Self(
                manifest
                    .packages
                    .iter()
                    .map(|package| {
                        (
                            format!("{}@v{}", package.crate_name, package.version),
                            complete_assets(&package.crate_name, &package.version),
                        )
                    })
                    .collect(),
            )
        }

        fn with(entries: &[(&str, Vec<String>)]) -> Self {
            Self(
                entries
                    .iter()
                    .map(|(key, assets)| ((*key).to_string(), assets.clone()))
                    .collect(),
            )
        }
    }

    /// One archive plus its checksum — the shape `ensure-release` ends up with.
    fn complete_assets(crate_name: &str, version: &str) -> Vec<String> {
        let archive = format!("{crate_name}-v{version}-x86_64-unknown-linux-gnu.tgz");
        vec![format!("{archive}.sha256"), archive]
    }

    fn manifest_with_channel(
        channel: Option<&str>,
        packages: &[(&str, &str)],
    ) -> ToolchainManifest {
        ToolchainManifest {
            schema: TOOLCHAIN_MANIFEST_SCHEMA.to_string(),
            toolchain: TOOLCHAIN_NAME.to_string(),
            version: "1.1.13".to_string(),
            channel: channel.map(str::to_string),
            created_at: None,
            packages: packages
                .iter()
                .map(|(crate_name, version)| ToolchainPackage {
                    crate_name: (*crate_name).to_string(),
                    bins: vec![(*crate_name).to_string()],
                    version: (*version).to_string(),
                    artifacts: None,
                })
                .collect(),
            extension_packs: None,
            components: None,
            gtc: None,
        }
    }

    #[test]
    fn release_gate_skips_dev_channel() {
        let manifest = manifest_with_channel(Some("dev"), &[("greentic-setup", "1.2.30516109579")]);
        // Empty checker: every release is "missing", so a firing gate would fail.
        verify_manifest_releases(&manifest, Some("dev"), &StubReleaseChecker::with(&[])).unwrap();
    }

    #[test]
    fn release_gate_skips_manifest_without_channel_or_stable_tag() {
        let manifest = manifest_with_channel(None, &[("greentic-setup", "1.1.31")]);
        verify_manifest_releases(&manifest, None, &StubReleaseChecker::with(&[])).unwrap();
    }

    #[test]
    fn release_gate_accepts_complete_stable_releases() {
        let manifest = manifest_with_channel(
            Some("stable"),
            &[("greentic-setup", "1.1.31"), ("greentic-start", "1.1.38")],
        );
        let checker = StubReleaseChecker::complete_for(&manifest);
        verify_manifest_releases(&manifest, Some("stable"), &checker).unwrap();
    }

    /// The regression this gate exists for: greentic-setup 1.1.31 was pinned
    /// while its release build was still running, so `:stable` moved onto a
    /// binary nobody could download.
    #[test]
    fn release_gate_rejects_pin_whose_release_does_not_exist() {
        let manifest = manifest_with_channel(
            Some("stable"),
            &[("greentic-setup", "1.1.31"), ("greentic-start", "1.1.38")],
        );
        let checker = StubReleaseChecker::with(&[(
            "greentic-start@v1.1.38",
            complete_assets("greentic-start", "1.1.38"),
        )]);
        let error = verify_manifest_releases(&manifest, Some("stable"), &checker)
            .unwrap_err()
            .to_string();
        assert!(error.contains("greentic-setup v1.1.31"), "{error}");
        assert!(error.contains("no GitHub release"), "{error}");
        assert!(!error.contains("greentic-start"), "{error}");
    }

    /// A `--from latest` manifest records `channel: "latest"`, yet `--tag stable`
    /// still moves the tag `gtc install` resolves. It must be gated.
    #[test]
    fn release_gate_fires_on_stable_tag_despite_latest_channel() {
        let manifest = manifest_with_channel(Some("latest"), &[("greentic-setup", "1.1.31")]);
        let error =
            verify_manifest_releases(&manifest, Some("stable"), &StubReleaseChecker::with(&[]))
                .unwrap_err()
                .to_string();
        assert!(error.contains("greentic-setup v1.1.31"), "{error}");
    }

    #[test]
    fn release_gate_rejects_release_with_no_archives_yet() {
        let manifest = manifest_with_channel(Some("stable"), &[("greentic-setup", "1.1.31")]);
        let checker = StubReleaseChecker::with(&[("greentic-setup@v1.1.31", Vec::new())]);
        let error = verify_manifest_releases(&manifest, Some("stable"), &checker)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no v1.1.31 archives yet"), "{error}");
    }

    #[test]
    fn release_gate_rejects_archive_missing_its_checksum() {
        let manifest = manifest_with_channel(Some("stable"), &[("greentic-setup", "1.1.31")]);
        let checker = StubReleaseChecker::with(&[(
            "greentic-setup@v1.1.31",
            vec![
                "greentic-setup-v1.1.31-x86_64-unknown-linux-gnu.tgz".to_string(),
                "greentic-setup-v1.1.31-x86_64-unknown-linux-gnu.tgz.sha256".to_string(),
                "greentic-setup-v1.1.31-aarch64-apple-darwin.tgz".to_string(),
            ],
        )]);
        let error = verify_manifest_releases(&manifest, Some("stable"), &checker)
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing .sha256"), "{error}");
        assert!(error.contains("aarch64-apple-darwin"), "{error}");
    }

    /// Multi-binary repos (greentic-mcp ships two) attach several archives per
    /// target; the rule is per-archive, not a fixed asset count.
    #[test]
    fn release_gate_accepts_multi_binary_release() {
        let manifest = manifest_with_channel(Some("stable"), &[("greentic-mcp", "1.1.1")]);
        let checker = StubReleaseChecker::with(&[(
            "greentic-mcp@v1.1.1",
            vec![
                "greentic-mcp-v1.1.1-x86_64-unknown-linux-gnu.tgz".to_string(),
                "greentic-mcp-v1.1.1-x86_64-unknown-linux-gnu.tgz.sha256".to_string(),
                "greentic-mcp-generator-v1.1.1-x86_64-pc-windows-msvc.zip".to_string(),
                "greentic-mcp-generator-v1.1.1-x86_64-pc-windows-msvc.zip.sha256".to_string(),
            ],
        )]);
        verify_manifest_releases(&manifest, Some("stable"), &checker).unwrap();
    }

    /// `snapshot --channel stable` passes no `--tag`, so the channel half of the
    /// predicate is what gates it. crates.io presence does not imply a finished
    /// release: greentic-pack publishes crates on its own `push: tags` trigger,
    /// independent of the release job.
    #[test]
    fn release_gate_fires_on_stable_channel_without_a_target_tag() {
        let manifest = manifest_with_channel(Some("stable"), &[("greentic-pack", "1.1.5")]);
        let error = verify_manifest_releases(&manifest, None, &StubReleaseChecker::with(&[]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("greentic-pack v1.1.5"), "{error}");
        assert!(error.contains("no GitHub release"), "{error}");
    }

    /// greentic-pack attaches unversioned `greentic-pack-<target>.tgz` binstall
    /// aliases with no checksums next to the canonical versioned set. Requiring
    /// a `.sha256` for every archive rejected every real pack release.
    #[test]
    fn release_gate_ignores_unversioned_binstall_aliases() {
        let manifest = manifest_with_channel(Some("stable"), &[("greentic-pack", "1.1.5")]);
        let checker = StubReleaseChecker::with(&[(
            "greentic-pack@v1.1.5",
            vec![
                "greentic-pack-v1.1.5-x86_64-unknown-linux-gnu.tgz".to_string(),
                "greentic-pack-v1.1.5-x86_64-unknown-linux-gnu.tgz.sha256".to_string(),
                // Alias, no checksum — must not be read as an unfinished upload.
                "greentic-pack-x86_64-unknown-linux-gnu.tgz".to_string(),
            ],
        )]);
        verify_manifest_releases(&manifest, Some("stable"), &checker).unwrap();
    }

    /// ...but aliases alone are not a usable release: binstall resolves the
    /// versioned names.
    #[test]
    fn release_gate_rejects_release_with_only_unversioned_aliases() {
        let manifest = manifest_with_channel(Some("stable"), &[("greentic-pack", "1.1.5")]);
        let checker = StubReleaseChecker::with(&[(
            "greentic-pack@v1.1.5",
            vec!["greentic-pack-x86_64-unknown-linux-gnu.tgz".to_string()],
        )]);
        let error = verify_manifest_releases(&manifest, Some("stable"), &checker)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no v1.1.5 archives yet"), "{error}");
    }

    #[test]
    fn release_response_404_means_absent() {
        assert_eq!(
            classify_release_response(reqwest::StatusCode::NOT_FOUND, "url", "{}".to_string())
                .unwrap(),
            None
        );
    }

    #[test]
    fn release_response_success_returns_body() {
        assert_eq!(
            classify_release_response(reqwest::StatusCode::OK, "url", "{}".to_string()).unwrap(),
            Some("{}".to_string())
        );
    }

    #[test]
    fn release_response_server_error_is_fatal() {
        // A 5xx must never be mistaken for "release absent" — that would let a
        // GitHub outage wave a bad manifest straight through the gate.
        assert!(
            classify_release_response(
                reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                "url",
                "boom".to_string()
            )
            .is_err()
        );
    }

    #[test]
    fn latest_dry_run_succeeds() {
        latest(ReleaseLatestArgs {
            repo: "ghcr.io/greenticai/greentic-versions/gtc".to_string(),
            token: None,
            dry_run: true,
            force: false,
        })
        .unwrap();
    }

    #[test]
    fn promote_dry_run_succeeds() {
        promote(ReleasePromoteArgs {
            release: "1.0.12".to_string(),
            tag: "stable".to_string(),
            repo: "ghcr.io/greenticai/greentic-versions/gtc".to_string(),
            token: None,
            dry_run: true,
            no_notify_updater: true,
        })
        .unwrap();
    }

    #[test]
    fn builds_toolchain_ref() {
        assert_eq!(
            toolchain_ref("ghcr.io/greenticai/greentic-versions/gtc", "stable"),
            "ghcr.io/greenticai/greentic-versions/gtc:stable"
        );
    }

    // -----------------------------------------------------------------------
    // should_notify_updater — stable-lane gate tests
    // -----------------------------------------------------------------------

    #[test]
    fn notify_gate_accepts_stable_plain_version() {
        assert!(should_notify_updater("1.1.2", "stable"));
    }

    #[test]
    fn notify_gate_accepts_stable_zero_version() {
        assert!(should_notify_updater("0.1.0", "stable"));
    }

    #[test]
    fn notify_gate_rejects_dev_channel() {
        assert!(!should_notify_updater("1.1.2", "dev"));
    }

    #[test]
    fn notify_gate_rejects_rnd_channel() {
        assert!(!should_notify_updater("1.1.2", "rnd"));
    }

    #[test]
    fn notify_gate_rejects_prerelease_version() {
        assert!(!should_notify_updater("1.2.0-dev.3", "stable"));
    }

    #[test]
    fn notify_gate_rejects_run_id_version_on_dev_channel() {
        // Run-id versions (e.g. 1.1.14995680637) are dev-lane artifacts and
        // always carry channel "dev". The channel check catches them.
        assert!(!should_notify_updater("1.1.14995680637", "dev"));
    }

    #[test]
    fn notify_gate_rejects_research_prerelease() {
        assert!(!should_notify_updater("1.3.0-research.1", "stable"));
    }

    #[test]
    fn notify_gate_rejects_build_metadata() {
        assert!(!should_notify_updater("1.1.2+build.42", "stable"));
    }

    #[test]
    fn notify_gate_rejects_empty_channel() {
        assert!(!should_notify_updater("1.1.2", ""));
    }

    #[test]
    fn notify_gate_rejects_unparseable_version() {
        assert!(!should_notify_updater("not-a-version", "stable"));
    }
}
