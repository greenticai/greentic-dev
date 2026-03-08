use std::fs;
use std::future::Future;
use std::io::IsTerminal;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use flate2::read::GzDecoder;
use greentic_distributor_client::oci_packs::{OciPackFetcher, PackFetchOptions, RegistryClient};
use oci_distribution::Reference;
use oci_distribution::client::{Client, ClientConfig, ClientProtocol, ImageData};
use oci_distribution::errors::OciDistributionError;
use oci_distribution::manifest::{IMAGE_MANIFEST_MEDIA_TYPE, OCI_IMAGE_MEDIA_TYPE};
use oci_distribution::secrets::RegistryAuth;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::Archive;
use zip::ZipArchive;

use crate::cli::InstallArgs;
use crate::cmd::tools;
use crate::i18n;

const CUSTOMERS_TOOLS_REPO: &str = "ghcr.io/greentic-biz/customers-tools";
const OCI_LAYER_JSON_MEDIA_TYPE: &str = "application/json";
const OAUTH_USER: &str = "oauth2";

pub fn run(args: InstallArgs) -> Result<()> {
    tools::install(false)?;
    let locale = i18n::select_locale(args.locale.as_deref());

    let Some(tenant) = args.tenant else {
        return Ok(());
    };

    let token = resolve_token(args.token, &locale)
        .context(i18n::t(&locale, "cli.install.error.tenant_requires_token"))?;

    let env = InstallEnv::detect(args.bin_dir, args.docs_dir, Some(locale))?;
    let installer = Installer::new(RealTenantManifestSource, RealHttpDownloader::default(), env);
    installer.install_tenant(&tenant, &token)
}

fn resolve_token(raw: Option<String>, locale: &str) -> Result<String> {
    resolve_token_with(
        raw,
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        || prompt_for_token(locale),
        locale,
    )
}

fn resolve_token_with<F>(
    raw: Option<String>,
    interactive: bool,
    prompt: F,
    locale: &str,
) -> Result<String>
where
    F: FnOnce() -> Result<String>,
{
    let Some(raw) = raw else {
        if interactive {
            return prompt();
        }
        bail!(
            "{}",
            i18n::t(locale, "cli.install.error.missing_token_non_interactive")
        );
    };
    if let Some(var) = raw.strip_prefix("env:") {
        let value = std::env::var(var).with_context(|| {
            i18n::tf(
                locale,
                "cli.install.error.env_token_resolve",
                &[("var", var.to_string())],
            )
        })?;
        if value.trim().is_empty() {
            bail!(
                "{}",
                i18n::tf(
                    locale,
                    "cli.install.error.env_token_empty",
                    &[("var", var.to_string())],
                )
            );
        }
        Ok(value)
    } else if raw.trim().is_empty() {
        if interactive {
            prompt()
        } else {
            bail!(
                "{}",
                i18n::t(locale, "cli.install.error.empty_token_non_interactive")
            );
        }
    } else {
        Ok(raw)
    }
}

fn prompt_for_token(locale: &str) -> Result<String> {
    let token = rpassword::prompt_password(i18n::t(locale, "cli.install.prompt.github_token"))
        .context(i18n::t(locale, "cli.install.error.read_token"))?;
    if token.trim().is_empty() {
        bail!("{}", i18n::t(locale, "cli.install.error.empty_token"));
    }
    Ok(token)
}

#[derive(Clone, Debug)]
struct InstallEnv {
    install_root: PathBuf,
    bin_dir: PathBuf,
    docs_dir: PathBuf,
    downloads_dir: PathBuf,
    manifests_dir: PathBuf,
    state_path: PathBuf,
    platform: Platform,
    locale: String,
}

impl InstallEnv {
    fn detect(
        bin_dir: Option<PathBuf>,
        docs_dir: Option<PathBuf>,
        locale: Option<String>,
    ) -> Result<Self> {
        let locale = locale.clone().unwrap_or_else(|| "en-US".to_string());
        let home = dirs::home_dir().context(i18n::t(&locale, "cli.install.error.home_dir"))?;
        let greentic_root = home.join(".greentic");
        let install_root = greentic_root.join("install");
        let bin_dir = match bin_dir {
            Some(path) => path,
            None => default_bin_dir(&home),
        };
        let docs_dir = docs_dir.unwrap_or_else(|| install_root.join("docs"));
        let downloads_dir = install_root.join("downloads");
        let manifests_dir = install_root.join("manifests");
        let state_path = install_root.join("state.json");
        Ok(Self {
            install_root,
            bin_dir,
            docs_dir,
            downloads_dir,
            manifests_dir,
            state_path,
            platform: Platform::detect()?,
            locale,
        })
    }

    fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            &self.install_root,
            &self.bin_dir,
            &self.docs_dir,
            &self.downloads_dir,
            &self.manifests_dir,
        ] {
            fs::create_dir_all(dir).with_context(|| {
                i18n::tf(
                    &self.locale,
                    "cli.install.error.create_dir",
                    &[("path", dir.display().to_string())],
                )
            })?;
        }
        Ok(())
    }
}

fn default_bin_dir(home: &Path) -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_HOME") {
        PathBuf::from(path).join("bin")
    } else {
        home.join(".cargo").join("bin")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Platform {
    os: String,
    arch: String,
}

impl Platform {
    fn detect() -> Result<Self> {
        let os = match std::env::consts::OS {
            "linux" => "linux",
            "windows" => "windows",
            "macos" => "macos",
            other => bail!(
                "{}",
                i18n::tf(
                    "en",
                    "cli.install.error.unsupported_os",
                    &[("os", other.to_string())],
                )
            ),
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x86_64",
            "aarch64" => "aarch64",
            other => bail!(
                "{}",
                i18n::tf(
                    "en",
                    "cli.install.error.unsupported_arch",
                    &[("arch", other.to_string())],
                )
            ),
        };
        Ok(Self {
            os: os.to_string(),
            arch: arch.to_string(),
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct TenantInstallManifest {
    #[serde(rename = "$schema", default)]
    schema: Option<String>,
    schema_version: String,
    tenant: String,
    #[serde(default)]
    tools: Vec<TenantToolDescriptor>,
    #[serde(default)]
    docs: Vec<TenantDocDescriptor>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct TenantToolEntry {
    #[serde(rename = "$schema", default)]
    schema: Option<String>,
    id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    install: ToolInstall,
    #[serde(default)]
    docs: Vec<String>,
    #[serde(default)]
    i18n: std::collections::BTreeMap<String, ToolTranslation>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct TenantDocEntry {
    #[serde(rename = "$schema", default)]
    schema: Option<String>,
    id: String,
    title: String,
    source: DocSource,
    download_file_name: String,
    #[serde(alias = "relative_path")]
    default_relative_path: String,
    #[serde(default)]
    i18n: std::collections::BTreeMap<String, DocTranslation>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SimpleTenantToolEntry {
    id: String,
    #[serde(default)]
    binary_name: Option<String>,
    targets: Vec<ReleaseTarget>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SimpleTenantDocEntry {
    url: String,
    #[serde(alias = "download_file_name")]
    file_name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
enum TenantToolDescriptor {
    Expanded(TenantToolEntry),
    Simple(SimpleTenantToolEntry),
    Ref(RemoteManifestRef),
    Id(String),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
enum TenantDocDescriptor {
    Expanded(TenantDocEntry),
    Simple(SimpleTenantDocEntry),
    Ref(RemoteManifestRef),
    Id(String),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RemoteManifestRef {
    id: String,
    #[serde(alias = "manifest_url")]
    url: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct ToolTranslation {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    docs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct DocTranslation {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    download_file_name: Option<String>,
    #[serde(default)]
    default_relative_path: Option<String>,
    #[serde(default)]
    source: Option<DocSource>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ToolInstall {
    #[serde(rename = "type")]
    install_type: String,
    binary_name: String,
    targets: Vec<ReleaseTarget>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ReleaseTarget {
    os: String,
    arch: String,
    url: String,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct DocSource {
    #[serde(rename = "type")]
    source_type: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubReleaseAsset {
    name: String,
    url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct InstallState {
    tenant: String,
    locale: String,
    manifest_path: String,
    installed_bins: Vec<String>,
    installed_docs: Vec<String>,
}

#[async_trait]
trait TenantManifestSource: Send + Sync {
    async fn fetch_manifest(&self, tenant: &str, token: &str) -> Result<Vec<u8>>;
}

#[async_trait]
trait Downloader: Send + Sync {
    async fn download(&self, url: &str, token: &str) -> Result<Vec<u8>>;
}

struct Installer<S, D> {
    source: S,
    downloader: D,
    env: InstallEnv,
}

impl<S, D> Installer<S, D>
where
    S: TenantManifestSource,
    D: Downloader,
{
    fn new(source: S, downloader: D, env: InstallEnv) -> Self {
        Self {
            source,
            downloader,
            env,
        }
    }

    fn install_tenant(&self, tenant: &str, token: &str) -> Result<()> {
        block_on_maybe_runtime(self.install_tenant_async(tenant, token))
    }

    async fn install_tenant_async(&self, tenant: &str, token: &str) -> Result<()> {
        self.env.ensure_dirs()?;
        let manifest_bytes = self.source.fetch_manifest(tenant, token).await?;
        let manifest: TenantInstallManifest = serde_json::from_slice(&manifest_bytes)
            .with_context(|| {
                i18n::tf(
                    &self.env.locale,
                    "cli.install.error.parse_tenant_manifest",
                    &[("tenant", tenant.to_string())],
                )
            })?;
        if manifest.tenant != tenant {
            bail!(
                "{}",
                i18n::tf(
                    &self.env.locale,
                    "cli.install.error.tenant_manifest_mismatch",
                    &[
                        ("tenant", tenant.to_string()),
                        ("manifest_tenant", manifest.tenant.clone())
                    ]
                )
            );
        }

        let mut installed_bins = Vec::new();
        let mut installed_tool_entries = Vec::new();
        for tool in &manifest.tools {
            let tool = self.resolve_tool(tool, token).await?;
            let path = self.install_tool(&tool, token).await?;
            installed_tool_entries.push((tool.id.clone(), path.clone()));
            installed_bins.push(path.display().to_string());
        }

        let mut installed_docs = Vec::new();
        let mut installed_doc_entries = Vec::new();
        for doc in &manifest.docs {
            let doc = self.resolve_doc(doc, token).await?;
            let path = self.install_doc(&doc, token).await?;
            installed_doc_entries.push((doc.id.clone(), path.clone()));
            installed_docs.push(path.display().to_string());
        }

        let manifest_path = self.env.manifests_dir.join(format!("tenant-{tenant}.json"));
        fs::write(&manifest_path, &manifest_bytes).with_context(|| {
            i18n::tf(
                &self.env.locale,
                "cli.install.error.write_file",
                &[("path", manifest_path.display().to_string())],
            )
        })?;
        let state = InstallState {
            tenant: tenant.to_string(),
            locale: self.env.locale.clone(),
            manifest_path: manifest_path.display().to_string(),
            installed_bins,
            installed_docs,
        };
        let state_json = serde_json::to_vec_pretty(&state).context(i18n::t(
            &self.env.locale,
            "cli.install.error.serialize_state",
        ))?;
        fs::write(&self.env.state_path, state_json).with_context(|| {
            i18n::tf(
                &self.env.locale,
                "cli.install.error.write_file",
                &[("path", self.env.state_path.display().to_string())],
            )
        })?;
        print_install_summary(
            &self.env.locale,
            &installed_tool_entries,
            &installed_doc_entries,
        );
        Ok(())
    }

    async fn resolve_tool(
        &self,
        tool: &TenantToolDescriptor,
        token: &str,
    ) -> Result<TenantToolEntry> {
        match tool {
            TenantToolDescriptor::Expanded(entry) => Ok(entry.clone()),
            TenantToolDescriptor::Simple(entry) => Ok(TenantToolEntry {
                schema: None,
                id: entry.id.clone(),
                name: entry.id.clone(),
                description: None,
                install: ToolInstall {
                    install_type: "release-binary".to_string(),
                    binary_name: entry
                        .binary_name
                        .clone()
                        .unwrap_or_else(|| entry.id.clone()),
                    targets: entry.targets.clone(),
                },
                docs: Vec::new(),
                i18n: std::collections::BTreeMap::new(),
            }),
            TenantToolDescriptor::Ref(reference) => {
                enforce_github_url(&reference.url)?;
                let bytes = self.downloader.download(&reference.url, token).await?;
                let manifest: TenantToolEntry =
                    serde_json::from_slice(&bytes).with_context(|| {
                        format!("failed to parse tool manifest `{}`", reference.url)
                    })?;
                if manifest.id != reference.id {
                    bail!(
                        "tool manifest mismatch: tenant referenced `{}` but manifest contained `{}`",
                        reference.id,
                        manifest.id
                    );
                }
                Ok(manifest)
            }
            TenantToolDescriptor::Id(id) => bail!(
                "tool id `{id}` requires a manifest URL; bare IDs are not supported by greentic-dev"
            ),
        }
    }

    async fn resolve_doc(&self, doc: &TenantDocDescriptor, token: &str) -> Result<TenantDocEntry> {
        match doc {
            TenantDocDescriptor::Expanded(entry) => Ok(entry.clone()),
            TenantDocDescriptor::Simple(entry) => Ok(TenantDocEntry {
                schema: None,
                id: entry.file_name.clone(),
                title: entry.file_name.clone(),
                source: DocSource {
                    source_type: "download".to_string(),
                    url: entry.url.clone(),
                },
                download_file_name: entry.file_name.clone(),
                default_relative_path: entry.file_name.clone(),
                i18n: std::collections::BTreeMap::new(),
            }),
            TenantDocDescriptor::Ref(reference) => {
                enforce_github_url(&reference.url)?;
                let bytes = self.downloader.download(&reference.url, token).await?;
                let manifest: TenantDocEntry = serde_json::from_slice(&bytes)
                    .with_context(|| format!("failed to parse doc manifest `{}`", reference.url))?;
                if manifest.id != reference.id {
                    bail!(
                        "doc manifest mismatch: tenant referenced `{}` but manifest contained `{}`",
                        reference.id,
                        manifest.id
                    );
                }
                Ok(manifest)
            }
            TenantDocDescriptor::Id(id) => bail!(
                "doc id `{id}` requires a manifest URL; bare IDs are not supported by greentic-dev"
            ),
        }
    }

    async fn install_tool(&self, tool: &TenantToolEntry, token: &str) -> Result<PathBuf> {
        let tool = apply_tool_locale(tool, &self.env.locale);
        if tool.install.install_type != "release-binary" {
            bail!(
                "tool `{}` has unsupported install type `{}`",
                tool.id,
                tool.install.install_type
            );
        }
        let target = select_release_target(&tool.install.targets, &self.env.platform)
            .with_context(|| format!("failed to select release target for `{}`", tool.id))?;
        enforce_github_url(&target.url)?;
        let bytes = self.downloader.download(&target.url, token).await?;
        if let Some(sha256) = &target.sha256 {
            verify_sha256(&bytes, sha256)
                .with_context(|| format!("checksum verification failed for `{}`", tool.id))?;
        }

        let target_name = binary_filename(&expected_binary_name(
            &tool.install.binary_name,
            &target.url,
        ));
        let staged_path =
            self.env
                .downloads_dir
                .join(format!("{}-{}", tool.id, file_name_hint(&target.url)));
        fs::write(&staged_path, &bytes)
            .with_context(|| format!("failed to write {}", staged_path.display()))?;

        let installed_path = if target.url.ends_with(".tar.gz") || target.url.ends_with(".tgz") {
            extract_tar_gz_binary(&bytes, &target_name, &self.env.bin_dir)?
        } else if target.url.ends_with(".zip") {
            extract_zip_binary(&bytes, &target_name, &self.env.bin_dir)?
        } else {
            let dest_path = self.env.bin_dir.join(&target_name);
            fs::write(&dest_path, &bytes)
                .with_context(|| format!("failed to write {}", dest_path.display()))?;
            dest_path
        };

        ensure_executable(&installed_path)?;
        Ok(installed_path)
    }

    async fn install_doc(&self, doc: &TenantDocEntry, token: &str) -> Result<PathBuf> {
        let doc = apply_doc_locale(doc, &self.env.locale);
        if doc.source.source_type != "download" {
            bail!(
                "doc `{}` has unsupported source type `{}`",
                doc.id,
                doc.source.source_type
            );
        }
        enforce_github_url(&doc.source.url)?;
        let relative = sanitize_relative_path(&doc.default_relative_path)?;
        let dest_path = self.env.docs_dir.join(relative);
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let bytes = self.downloader.download(&doc.source.url, token).await?;
        fs::write(&dest_path, &bytes)
            .with_context(|| format!("failed to write {}", dest_path.display()))?;
        Ok(dest_path)
    }
}

fn block_on_maybe_runtime<F, T>(future: F) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(future))
    } else {
        let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
        rt.block_on(future)
    }
}

fn apply_tool_locale(tool: &TenantToolEntry, locale: &str) -> TenantToolEntry {
    let mut localized = tool.clone();
    if let Some(translation) = resolve_translation(&tool.i18n, locale) {
        if let Some(name) = &translation.name {
            localized.name = name.clone();
        }
        if let Some(description) = &translation.description {
            localized.description = Some(description.clone());
        }
        if let Some(docs) = &translation.docs {
            localized.docs = docs.clone();
        }
    }
    localized
}

fn apply_doc_locale(doc: &TenantDocEntry, locale: &str) -> TenantDocEntry {
    let mut localized = doc.clone();
    if let Some(translation) = resolve_translation(&doc.i18n, locale) {
        if let Some(title) = &translation.title {
            localized.title = title.clone();
        }
        if let Some(download_file_name) = &translation.download_file_name {
            localized.download_file_name = download_file_name.clone();
        }
        if let Some(default_relative_path) = &translation.default_relative_path {
            localized.default_relative_path = default_relative_path.clone();
        }
        if let Some(source) = &translation.source {
            localized.source = source.clone();
        }
    }
    localized
}

fn resolve_translation<'a, T>(
    map: &'a std::collections::BTreeMap<String, T>,
    locale: &str,
) -> Option<&'a T> {
    if let Some(exact) = map.get(locale) {
        return Some(exact);
    }
    let lang = locale.split(['-', '_']).next().unwrap_or(locale);
    map.get(lang)
}

fn binary_filename(name: &str) -> String {
    if cfg!(windows) && !name.ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn file_name_hint(url: &str) -> String {
    url.rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or("download.bin")
        .to_string()
}

fn expected_binary_name(configured: &str, url: &str) -> String {
    let fallback = configured.to_string();
    let asset = file_name_hint(url);
    let stem = asset
        .strip_suffix(".tar.gz")
        .or_else(|| asset.strip_suffix(".tgz"))
        .or_else(|| asset.strip_suffix(".zip"))
        .unwrap_or(asset.as_str());
    if let Some(prefix) = stem
        .strip_suffix("-x86_64-unknown-linux-gnu")
        .or_else(|| stem.strip_suffix("-aarch64-unknown-linux-gnu"))
        .or_else(|| stem.strip_suffix("-x86_64-apple-darwin"))
        .or_else(|| stem.strip_suffix("-aarch64-apple-darwin"))
        .or_else(|| stem.strip_suffix("-x86_64-pc-windows-msvc"))
        .or_else(|| stem.strip_suffix("-aarch64-pc-windows-msvc"))
    {
        return strip_version_suffix(prefix);
    }
    fallback
}

fn strip_version_suffix(name: &str) -> String {
    let Some((prefix, last)) = name.rsplit_once('-') else {
        return name.to_string();
    };
    if is_version_segment(last) {
        prefix.to_string()
    } else {
        name.to_string()
    }
}

fn is_version_segment(segment: &str) -> bool {
    let trimmed = segment.strip_prefix('v').unwrap_or(segment);
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == '_' || ch == '-')
        && trimmed.chars().any(|ch| ch.is_ascii_digit())
}

fn select_release_target<'a>(
    targets: &'a [ReleaseTarget],
    platform: &Platform,
) -> Result<&'a ReleaseTarget> {
    targets
        .iter()
        .find(|target| target.os == platform.os && target.arch == platform.arch)
        .ok_or_else(|| anyhow!("no target for {} / {}", platform.os, platform.arch))
}

fn verify_sha256(bytes: &[u8], expected: &str) -> Result<()> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected.to_ascii_lowercase() {
        bail!("sha256 mismatch: expected {expected}, got {actual}");
    }
    Ok(())
}

fn sanitize_relative_path(path: &str) -> Result<PathBuf> {
    let pb = PathBuf::from(path);
    if pb.is_absolute() {
        bail!("absolute doc install paths are not allowed");
    }
    for component in pb.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            bail!("doc install path must stay within the docs directory");
        }
    }
    Ok(pb)
}

fn extract_tar_gz_binary(bytes: &[u8], binary_name: &str, dest_dir: &Path) -> Result<PathBuf> {
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = Archive::new(decoder);
    let mut fallback: Option<PathBuf> = None;
    let mut extracted = Vec::new();
    for entry in archive.entries().context("failed to read tar.gz archive")? {
        let mut entry = entry.context("failed to read tar.gz archive entry")?;
        let path = entry.path().context("failed to read tar.gz entry path")?;
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let name = name.to_string();
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let out_path = dest_dir.join(&name);
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .with_context(|| format!("failed to extract `{name}` from tar.gz"))?;
        fs::write(&out_path, buf)
            .with_context(|| format!("failed to write {}", out_path.display()))?;
        extracted.push(out_path.clone());
        if name == binary_name {
            return Ok(out_path);
        }
        if fallback.is_none() && archive_name_matches(binary_name, &name) {
            fallback = Some(out_path);
        }
    }
    if let Some(path) = fallback {
        return Ok(path);
    }
    if let Some(path) = extracted.into_iter().next() {
        return Ok(path);
    }
    let (debug_dir, entries) = dump_tar_gz_debug(bytes, binary_name)?;
    bail!(
        "archive did not contain `{binary_name}`. extracted debug dump to `{}` with entries: {}",
        debug_dir.display(),
        entries.join(", ")
    );
}

fn extract_zip_binary(bytes: &[u8], binary_name: &str, dest_dir: &Path) -> Result<PathBuf> {
    let cursor = Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor).context("failed to open zip archive")?;
    let mut fallback: Option<PathBuf> = None;
    let mut extracted = Vec::new();
    for idx in 0..archive.len() {
        let mut file = archive
            .by_index(idx)
            .context("failed to read zip archive entry")?;
        if file.is_dir() {
            continue;
        }
        let Some(name) = Path::new(file.name())
            .file_name()
            .and_then(|name| name.to_str())
        else {
            continue;
        };
        let name = name.to_string();
        let out_path = dest_dir.join(&name);
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .with_context(|| format!("failed to extract `{name}` from zip"))?;
        fs::write(&out_path, buf)
            .with_context(|| format!("failed to write {}", out_path.display()))?;
        extracted.push(out_path.clone());
        if name == binary_name {
            return Ok(out_path);
        }
        if fallback.is_none() && archive_name_matches(binary_name, &name) {
            fallback = Some(out_path);
        }
    }
    if let Some(path) = fallback {
        return Ok(path);
    }
    if let Some(path) = extracted.into_iter().next() {
        return Ok(path);
    }
    let (debug_dir, entries) = dump_zip_debug(bytes, binary_name)?;
    bail!(
        "archive did not contain `{binary_name}`. extracted debug dump to `{}` with entries: {}",
        debug_dir.display(),
        entries.join(", ")
    );
}

fn archive_name_matches(expected: &str, actual: &str) -> bool {
    let expected = expected.strip_suffix(".exe").unwrap_or(expected);
    let actual = actual.strip_suffix(".exe").unwrap_or(actual);
    actual == expected
        || actual.starts_with(&format!("{expected}-"))
        || actual.starts_with(&format!("{expected}_"))
        || strip_version_suffix(actual) == expected
}

fn print_install_summary(locale: &str, tools: &[(String, PathBuf)], docs: &[(String, PathBuf)]) {
    println!("{}", i18n::t(locale, "cli.install.summary.tools"));
    for (id, path) in tools {
        println!(
            "{}",
            i18n::tf(
                locale,
                "cli.install.summary.tool_item",
                &[("id", id.clone()), ("path", path.display().to_string()),],
            )
        );
    }
    println!("{}", i18n::t(locale, "cli.install.summary.docs"));
    for (id, path) in docs {
        println!(
            "{}",
            i18n::tf(
                locale,
                "cli.install.summary.doc_item",
                &[("id", id.clone()), ("path", path.display().to_string()),],
            )
        );
    }
}

fn dump_tar_gz_debug(bytes: &[u8], binary_name: &str) -> Result<(PathBuf, Vec<String>)> {
    let debug_dir = create_archive_debug_dir(binary_name)?;
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = Archive::new(decoder);
    let mut entries = Vec::new();
    for entry in archive
        .entries()
        .context("failed to read tar.gz archive for debug dump")?
    {
        let mut entry = entry.context("failed to read tar.gz archive entry for debug dump")?;
        let path = entry
            .path()
            .context("failed to read tar.gz entry path for debug dump")?
            .into_owned();
        let display = path.display().to_string();
        entries.push(display.clone());
        if let Some(relative) = safe_archive_relative_path(&path) {
            let out_path = debug_dir.join(relative);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            if entry.header().entry_type().is_dir() {
                fs::create_dir_all(&out_path)
                    .with_context(|| format!("failed to create {}", out_path.display()))?;
            } else if entry.header().entry_type().is_file() {
                let mut buf = Vec::new();
                entry
                    .read_to_end(&mut buf)
                    .with_context(|| format!("failed to extract `{display}` for debug dump"))?;
                fs::write(&out_path, buf)
                    .with_context(|| format!("failed to write {}", out_path.display()))?;
            }
        }
    }
    Ok((debug_dir, entries))
}

fn dump_zip_debug(bytes: &[u8], binary_name: &str) -> Result<(PathBuf, Vec<String>)> {
    let debug_dir = create_archive_debug_dir(binary_name)?;
    let cursor = Cursor::new(bytes);
    let mut archive =
        ZipArchive::new(cursor).context("failed to open zip archive for debug dump")?;
    let mut entries = Vec::new();
    for idx in 0..archive.len() {
        let mut file = archive
            .by_index(idx)
            .context("failed to read zip archive entry for debug dump")?;
        let path = PathBuf::from(file.name());
        let display = path.display().to_string();
        entries.push(display.clone());
        if let Some(relative) = safe_archive_relative_path(&path) {
            let out_path = debug_dir.join(relative);
            if file.is_dir() {
                fs::create_dir_all(&out_path)
                    .with_context(|| format!("failed to create {}", out_path.display()))?;
            } else {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("failed to create {}", parent.display()))?;
                }
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)
                    .with_context(|| format!("failed to extract `{display}` for debug dump"))?;
                fs::write(&out_path, buf)
                    .with_context(|| format!("failed to write {}", out_path.display()))?;
            }
        }
    }
    Ok((debug_dir, entries))
}

fn create_archive_debug_dir(binary_name: &str) -> Result<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time before unix epoch")?
        .as_millis();
    let dir = std::env::temp_dir().join(format!("greentic-dev-debug-{binary_name}-{stamp}"));
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    Ok(dir)
}

fn safe_archive_relative_path(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn ensure_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .with_context(|| format!("failed to read {}", path.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)
            .with_context(|| format!("failed to set executable bit on {}", path.display()))?;
    }
    Ok(())
}

fn enforce_github_url(url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url).with_context(|| format!("invalid URL `{url}`"))?;
    let Some(host) = parsed.host_str() else {
        bail!("URL `{url}` does not include a host");
    };
    let allowed = host == "github.com"
        || host.ends_with(".github.com")
        || host == "raw.githubusercontent.com"
        || host.ends_with(".githubusercontent.com")
        || host == "127.0.0.1"
        || host == "localhost";
    if !allowed {
        bail!("only GitHub-hosted assets are supported, got `{host}`");
    }
    Ok(())
}

struct RealHttpDownloader {
    client: reqwest::Client,
}

impl Default for RealHttpDownloader {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(format!("greentic-dev/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build HTTP client");
        Self { client }
    }
}

#[async_trait]
impl Downloader for RealHttpDownloader {
    async fn download(&self, url: &str, token: &str) -> Result<Vec<u8>> {
        let response =
            if let Some(asset_api_url) = self.resolve_github_asset_api_url(url, token).await? {
                self.client
                    .get(asset_api_url)
                    .bearer_auth(token)
                    .header(reqwest::header::ACCEPT, "application/octet-stream")
                    .send()
                    .await
                    .with_context(|| format!("failed to download `{url}`"))?
            } else {
                self.client
                    .get(url)
                    .bearer_auth(token)
                    .send()
                    .await
                    .with_context(|| format!("failed to download `{url}`"))?
            }
            .error_for_status()
            .with_context(|| format!("download failed for `{url}`"))?;
        let bytes = response
            .bytes()
            .await
            .with_context(|| format!("failed to read response body from `{url}`"))?;
        Ok(bytes.to_vec())
    }
}

impl RealHttpDownloader {
    async fn resolve_github_asset_api_url(&self, url: &str, token: &str) -> Result<Option<String>> {
        let Some(spec) = parse_github_release_url(url) else {
            return Ok(None);
        };
        let api_url = format!(
            "https://api.github.com/repos/{}/{}/releases/tags/{}",
            spec.owner, spec.repo, spec.tag
        );
        let release = self
            .client
            .get(api_url)
            .bearer_auth(token)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .with_context(|| format!("failed to resolve GitHub release for `{url}`"))?
            .error_for_status()
            .with_context(|| format!("failed to resolve GitHub release for `{url}`"))?
            .json::<GithubRelease>()
            .await
            .with_context(|| format!("failed to parse GitHub release metadata for `{url}`"))?;
        let Some(asset) = release
            .assets
            .into_iter()
            .find(|asset| asset.name == spec.asset_name)
        else {
            bail!(
                "download failed for `{url}`: release asset `{}` not found on tag `{}`",
                spec.asset_name,
                spec.tag
            );
        };
        Ok(Some(asset.url))
    }
}

struct GithubReleaseUrlSpec {
    owner: String,
    repo: String,
    tag: String,
    asset_name: String,
}

fn parse_github_release_url(url: &str) -> Option<GithubReleaseUrlSpec> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if parsed.host_str()? != "github.com" {
        return None;
    }
    let segments = parsed.path_segments()?.collect::<Vec<_>>();
    if segments.len() < 6 {
        return None;
    }
    if segments[2] != "releases" || segments[3] != "download" {
        return None;
    }
    Some(GithubReleaseUrlSpec {
        owner: segments[0].to_string(),
        repo: segments[1].to_string(),
        tag: segments[4].to_string(),
        asset_name: segments[5..].join("/"),
    })
}

#[derive(Clone)]
struct AuthRegistryClient {
    inner: Client,
    token: String,
}

#[async_trait]
impl RegistryClient for AuthRegistryClient {
    fn default_client() -> Self {
        let config = ClientConfig {
            protocol: ClientProtocol::Https,
            ..Default::default()
        };
        Self {
            inner: Client::new(config),
            token: String::new(),
        }
    }

    async fn pull(
        &self,
        reference: &Reference,
        accepted_manifest_types: &[&str],
    ) -> Result<greentic_distributor_client::oci_packs::PulledImage, OciDistributionError> {
        let image = self
            .inner
            .pull(
                reference,
                &RegistryAuth::Basic(OAUTH_USER.to_string(), self.token.clone()),
                accepted_manifest_types.to_vec(),
            )
            .await?;
        Ok(convert_image(image))
    }
}

fn convert_image(image: ImageData) -> greentic_distributor_client::oci_packs::PulledImage {
    let layers = image
        .layers
        .into_iter()
        .map(|layer| {
            let digest = format!("sha256:{}", layer.sha256_digest());
            greentic_distributor_client::oci_packs::PulledLayer {
                media_type: layer.media_type,
                data: layer.data,
                digest: Some(digest),
            }
        })
        .collect();
    greentic_distributor_client::oci_packs::PulledImage {
        digest: image.digest,
        layers,
    }
}

#[derive(Default)]
struct RealTenantManifestSource;

#[async_trait]
impl TenantManifestSource for RealTenantManifestSource {
    async fn fetch_manifest(&self, tenant: &str, token: &str) -> Result<Vec<u8>> {
        let opts = PackFetchOptions {
            allow_tags: true,
            accepted_manifest_types: vec![
                OCI_IMAGE_MEDIA_TYPE.to_string(),
                IMAGE_MANIFEST_MEDIA_TYPE.to_string(),
            ],
            accepted_layer_media_types: vec![OCI_LAYER_JSON_MEDIA_TYPE.to_string()],
            preferred_layer_media_types: vec![OCI_LAYER_JSON_MEDIA_TYPE.to_string()],
            ..Default::default()
        };
        let client = AuthRegistryClient {
            inner: Client::new(ClientConfig {
                protocol: ClientProtocol::Https,
                ..Default::default()
            }),
            token: token.to_string(),
        };
        let fetcher = OciPackFetcher::with_client(client, opts);
        let reference = format!("{CUSTOMERS_TOOLS_REPO}/{tenant}:latest");
        let resolved = match fetcher.fetch_pack_to_cache(&reference).await {
            Ok(resolved) => resolved,
            Err(err) => {
                let msg = err.to_string();
                if msg.contains("manifest unknown") {
                    return Err(anyhow!(
                        "tenant manifest not found at `{reference}`. Check that the tenant slug is correct and that the OCI artifact has been published with tag `latest`."
                    ));
                }
                return Err(err)
                    .with_context(|| format!("failed to pull tenant OCI manifest `{reference}`"));
            }
        };
        fs::read(&resolved.path).with_context(|| {
            format!(
                "failed to read cached OCI manifest {}",
                resolved.path.display()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::collections::HashMap;
    use tempfile::TempDir;

    struct FakeTenantManifestSource {
        manifest: Vec<u8>,
    }

    #[async_trait]
    impl TenantManifestSource for FakeTenantManifestSource {
        async fn fetch_manifest(&self, _tenant: &str, _token: &str) -> Result<Vec<u8>> {
            Ok(self.manifest.clone())
        }
    }

    struct FakeDownloader {
        responses: HashMap<String, Vec<u8>>,
    }

    #[async_trait]
    impl Downloader for FakeDownloader {
        async fn download(&self, url: &str, token: &str) -> Result<Vec<u8>> {
            assert_eq!(token, "secret-token");
            self.responses
                .get(url)
                .cloned()
                .ok_or_else(|| anyhow!("unexpected URL {url}"))
        }
    }

    fn test_env(temp: &TempDir) -> Result<InstallEnv> {
        Ok(InstallEnv {
            install_root: temp.path().join("install"),
            bin_dir: temp.path().join("bin"),
            docs_dir: temp.path().join("docs"),
            downloads_dir: temp.path().join("downloads"),
            manifests_dir: temp.path().join("manifests"),
            state_path: temp.path().join("install/state.json"),
            platform: Platform {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
            },
            locale: "en-US".to_string(),
        })
    }

    fn expanded_manifest(tool_url: &str, doc_url: &str, tar_sha: &str, doc_path: &str) -> Vec<u8> {
        serde_json::to_vec(&TenantInstallManifest {
            schema: Some("https://raw.githubusercontent.com/greenticai/customers-tools/main/schemas/tenant-tools.schema.json".to_string()),
            schema_version: "1".to_string(),
            tenant: "acme".to_string(),
            tools: vec![TenantToolDescriptor::Expanded(TenantToolEntry {
                schema: Some(
                    "https://raw.githubusercontent.com/greenticai/customers-tools/main/schemas/tool.schema.json".to_string(),
                ),
                id: "greentic-x-cli".to_string(),
                name: "Greentic X CLI".to_string(),
                description: Some("CLI".to_string()),
                install: ToolInstall {
                    install_type: "release-binary".to_string(),
                    binary_name: "greentic-x".to_string(),
                    targets: vec![ReleaseTarget {
                        os: "linux".to_string(),
                        arch: "x86_64".to_string(),
                        url: tool_url.to_string(),
                        sha256: Some(tar_sha.to_string()),
                    }],
                },
                docs: vec!["acme-onboarding".to_string()],
                i18n: std::collections::BTreeMap::new(),
            })],
            docs: vec![TenantDocDescriptor::Expanded(TenantDocEntry {
                schema: Some(
                    "https://raw.githubusercontent.com/greenticai/customers-tools/main/schemas/doc.schema.json".to_string(),
                ),
                id: "acme-onboarding".to_string(),
                title: "Acme onboarding".to_string(),
                source: DocSource {
                    source_type: "download".to_string(),
                    url: doc_url.to_string(),
                },
                download_file_name: "onboarding.md".to_string(),
                default_relative_path: doc_path.to_string(),
                i18n: std::collections::BTreeMap::new(),
            })],
        })
        .unwrap()
    }

    fn referenced_manifest(tool_manifest_url: &str, doc_manifest_url: &str) -> Vec<u8> {
        serde_json::to_vec(&TenantInstallManifest {
            schema: Some("https://raw.githubusercontent.com/greenticai/customers-tools/main/schemas/tenant-tools.schema.json".to_string()),
            schema_version: "1".to_string(),
            tenant: "acme".to_string(),
            tools: vec![TenantToolDescriptor::Ref(RemoteManifestRef {
                id: "greentic-x-cli".to_string(),
                url: tool_manifest_url.to_string(),
            })],
            docs: vec![TenantDocDescriptor::Ref(RemoteManifestRef {
                id: "acme-onboarding".to_string(),
                url: doc_manifest_url.to_string(),
            })],
        })
        .unwrap()
    }

    fn tar_gz_with_binary(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o755);
            header.set_size(contents.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, name, Cursor::new(contents))
                .unwrap();
            builder.finish().unwrap();
        }
        let mut out = Vec::new();
        {
            let mut encoder =
                flate2::write::GzEncoder::new(&mut out, flate2::Compression::default());
            std::io::copy(&mut Cursor::new(tar_buf), &mut encoder).unwrap();
            encoder.finish().unwrap();
        }
        out
    }

    #[test]
    fn selects_matching_target() -> Result<()> {
        let platform = Platform {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        };
        let targets = vec![
            ReleaseTarget {
                os: "windows".to_string(),
                arch: "x86_64".to_string(),
                url: "https://github.com/x.zip".to_string(),
                sha256: Some("a".repeat(64)),
            },
            ReleaseTarget {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
                url: "https://github.com/y.tar.gz".to_string(),
                sha256: Some("b".repeat(64)),
            },
        ];
        let selected = select_release_target(&targets, &platform)?;
        assert_eq!(selected.url, "https://github.com/y.tar.gz");
        Ok(())
    }

    #[test]
    fn checksum_verification_reports_failure() {
        let err = verify_sha256(b"abc", &"0".repeat(64)).unwrap_err();
        assert!(format!("{err}").contains("sha256 mismatch"));
    }

    #[test]
    fn resolve_token_prompts_when_missing_in_interactive_mode() -> Result<()> {
        let token = resolve_token_with(None, true, || Ok("secret-token".to_string()), "en")?;
        assert_eq!(token, "secret-token");
        Ok(())
    }

    #[test]
    fn resolve_token_errors_when_missing_in_non_interactive_mode() {
        let err = resolve_token_with(None, false, || Ok("unused".to_string()), "en").unwrap_err();
        assert!(format!("{err}").contains("no interactive terminal"));
    }

    #[test]
    fn extracts_tar_gz_binary() -> Result<()> {
        let temp = TempDir::new()?;
        let archive = tar_gz_with_binary("greentic-x", b"hello");
        let out = extract_tar_gz_binary(&archive, "greentic-x", temp.path())?;
        assert_eq!(out, temp.path().join("greentic-x"));
        assert_eq!(fs::read(&out)?, b"hello");
        Ok(())
    }

    #[test]
    fn tenant_install_happy_path_writes_binary_doc_manifest_and_state() -> Result<()> {
        let temp = TempDir::new()?;
        let tool_archive = tar_gz_with_binary("greentic-x", b"bin");
        let sha = format!("{:x}", Sha256::digest(&tool_archive));
        let tool_url =
            "https://github.com/acme/releases/download/v1.2.3/greentic-x-linux-x86_64.tar.gz";
        let doc_url = "https://raw.githubusercontent.com/acme/docs/main/onboarding.md";
        let manifest = expanded_manifest(tool_url, doc_url, &sha, "acme/onboarding/README.md");

        let installer = Installer::new(
            FakeTenantManifestSource { manifest },
            FakeDownloader {
                responses: HashMap::from([
                    (tool_url.to_string(), tool_archive.clone()),
                    (doc_url.to_string(), b"# onboarding\n".to_vec()),
                ]),
            },
            test_env(&temp)?,
        );
        installer.install_tenant("acme", "secret-token")?;

        assert_eq!(fs::read(temp.path().join("bin/greentic-x"))?, b"bin");
        assert_eq!(
            fs::read_to_string(temp.path().join("docs/acme/onboarding/README.md"))?,
            "# onboarding\n"
        );
        assert!(temp.path().join("manifests/tenant-acme.json").exists());
        assert!(temp.path().join("install/state.json").exists());
        Ok(())
    }

    #[test]
    fn install_rejects_path_traversal_in_docs() -> Result<()> {
        let temp = TempDir::new()?;
        let archive = tar_gz_with_binary("greentic-x", b"bin");
        let sha = format!("{:x}", Sha256::digest(&archive));
        let tool_url =
            "https://github.com/acme/releases/download/v1.2.3/greentic-x-linux-x86_64.tar.gz";
        let doc_url = "https://raw.githubusercontent.com/acme/docs/main/onboarding.md";
        let manifest = expanded_manifest(tool_url, doc_url, &sha, "../escape.md");
        let installer = Installer::new(
            FakeTenantManifestSource { manifest },
            FakeDownloader {
                responses: HashMap::from([
                    (tool_url.to_string(), archive),
                    (doc_url.to_string(), b"# onboarding\n".to_vec()),
                ]),
            },
            test_env(&temp)?,
        );
        let err = installer
            .install_tenant("acme", "secret-token")
            .unwrap_err();
        assert!(format!("{err}").contains("docs directory"));
        Ok(())
    }

    #[test]
    fn tenant_install_resolves_tool_and_doc_manifests_by_url() -> Result<()> {
        let temp = TempDir::new()?;
        let tool_archive = tar_gz_with_binary("greentic-x", b"bin");
        let sha = format!("{:x}", Sha256::digest(&tool_archive));
        let tool_url =
            "https://github.com/acme/releases/download/v1.2.3/greentic-x-linux-x86_64.tar.gz";
        let doc_url = "https://raw.githubusercontent.com/acme/docs/main/onboarding.md";
        let tool_manifest_url = "https://raw.githubusercontent.com/greenticai/customers-tools/main/tools/greentic-x-cli/manifest.json";
        let doc_manifest_url = "https://raw.githubusercontent.com/greenticai/customers-tools/main/docs/acme-onboarding.json";
        let tenant_manifest = referenced_manifest(tool_manifest_url, doc_manifest_url);
        let tool_manifest = serde_json::to_vec(&TenantToolEntry {
            schema: Some(
                "https://raw.githubusercontent.com/greenticai/customers-tools/main/schemas/tool.schema.json".to_string(),
            ),
            id: "greentic-x-cli".to_string(),
            name: "Greentic X CLI".to_string(),
            description: Some("CLI".to_string()),
            install: ToolInstall {
                install_type: "release-binary".to_string(),
                binary_name: "greentic-x".to_string(),
                targets: vec![ReleaseTarget {
                    os: "linux".to_string(),
                    arch: "x86_64".to_string(),
                    url: tool_url.to_string(),
                    sha256: Some(sha.clone()),
                }],
            },
            docs: vec!["acme-onboarding".to_string()],
            i18n: std::collections::BTreeMap::new(),
        })?;
        let doc_manifest = serde_json::to_vec(&TenantDocEntry {
            schema: Some(
                "https://raw.githubusercontent.com/greenticai/customers-tools/main/schemas/doc.schema.json".to_string(),
            ),
            id: "acme-onboarding".to_string(),
            title: "Acme onboarding".to_string(),
            source: DocSource {
                source_type: "download".to_string(),
                url: doc_url.to_string(),
            },
            download_file_name: "onboarding.md".to_string(),
            default_relative_path: "acme/onboarding/README.md".to_string(),
            i18n: std::collections::BTreeMap::new(),
        })?;

        let installer = Installer::new(
            FakeTenantManifestSource {
                manifest: tenant_manifest,
            },
            FakeDownloader {
                responses: HashMap::from([
                    (tool_manifest_url.to_string(), tool_manifest),
                    (doc_manifest_url.to_string(), doc_manifest),
                    (tool_url.to_string(), tool_archive),
                    (doc_url.to_string(), b"# onboarding\n".to_vec()),
                ]),
            },
            test_env(&temp)?,
        );
        installer.install_tenant("acme", "secret-token")?;
        assert_eq!(fs::read(temp.path().join("bin/greentic-x"))?, b"bin");
        assert_eq!(
            fs::read_to_string(temp.path().join("docs/acme/onboarding/README.md"))?,
            "# onboarding\n"
        );
        Ok(())
    }

    #[test]
    fn locale_uses_language_specific_doc_translation() -> Result<()> {
        let temp = TempDir::new()?;
        let tool_archive = tar_gz_with_binary("greentic-x", b"bin");
        let sha = format!("{:x}", Sha256::digest(&tool_archive));
        let en_doc_url = "https://raw.githubusercontent.com/acme/docs/main/onboarding.md";
        let nl_doc_url = "https://raw.githubusercontent.com/acme/docs/main/onboarding.nl.md";
        let manifest = serde_json::to_vec(&TenantInstallManifest {
            schema: None,
            schema_version: "1".to_string(),
            tenant: "acme".to_string(),
            tools: vec![TenantToolDescriptor::Expanded(TenantToolEntry {
                schema: None,
                id: "greentic-x-cli".to_string(),
                name: "Greentic X CLI".to_string(),
                description: None,
                install: ToolInstall {
                    install_type: "release-binary".to_string(),
                    binary_name: "greentic-x".to_string(),
                    targets: vec![ReleaseTarget {
                        os: "linux".to_string(),
                        arch: "x86_64".to_string(),
                        url: "https://github.com/acme/releases/download/v1.2.3/greentic-x-linux-x86_64.tar.gz".to_string(),
                        sha256: Some(sha),
                    }],
                },
                docs: vec!["acme-onboarding".to_string()],
                i18n: std::collections::BTreeMap::new(),
            })],
            docs: vec![TenantDocDescriptor::Expanded(TenantDocEntry {
                schema: None,
                id: "acme-onboarding".to_string(),
                title: "Acme onboarding".to_string(),
                source: DocSource {
                    source_type: "download".to_string(),
                    url: en_doc_url.to_string(),
                },
                download_file_name: "onboarding.md".to_string(),
                default_relative_path: "acme/onboarding/README.md".to_string(),
                i18n: std::collections::BTreeMap::from([(
                    "nl".to_string(),
                    DocTranslation {
                        title: Some("Acme onboarding NL".to_string()),
                        download_file_name: Some("onboarding.nl.md".to_string()),
                        default_relative_path: Some("acme/onboarding/README.nl.md".to_string()),
                        source: Some(DocSource {
                            source_type: "download".to_string(),
                            url: nl_doc_url.to_string(),
                        }),
                    },
                )]),
            })],
        })?;
        let mut env = test_env(&temp)?;
        env.locale = "nl".to_string();
        let installer = Installer::new(
            FakeTenantManifestSource { manifest },
            FakeDownloader {
                responses: HashMap::from([
                    (
                        "https://github.com/acme/releases/download/v1.2.3/greentic-x-linux-x86_64.tar.gz".to_string(),
                        tool_archive,
                    ),
                    (en_doc_url.to_string(), b"# onboarding en\n".to_vec()),
                    (nl_doc_url.to_string(), b"# onboarding nl\n".to_vec()),
                ]),
            },
            env,
        );
        installer.install_tenant("acme", "secret-token")?;
        assert_eq!(
            fs::read_to_string(temp.path().join("docs/acme/onboarding/README.nl.md"))?,
            "# onboarding nl\n"
        );
        Ok(())
    }

    #[test]
    fn tenant_install_accepts_simple_manifest_shape() -> Result<()> {
        let temp = TempDir::new()?;
        let tool_archive = tar_gz_with_binary("greentic-fast2flow", b"bin");
        let tool_url = "https://github.com/greentic-biz/greentic-fast2flow/releases/download/v0.4.1/greentic-fast2flow-v0.4.1-x86_64-unknown-linux-gnu.tar.gz";
        let doc_url =
            "https://raw.githubusercontent.com/greentic-biz/greentic-fast2flow/master/README.md";
        let manifest = serde_json::to_vec(&TenantInstallManifest {
            schema: None,
            schema_version: "1".to_string(),
            tenant: "3point".to_string(),
            tools: vec![TenantToolDescriptor::Simple(SimpleTenantToolEntry {
                id: "greentic-fast2flow".to_string(),
                binary_name: None,
                targets: vec![ReleaseTarget {
                    os: "linux".to_string(),
                    arch: "x86_64".to_string(),
                    url: tool_url.to_string(),
                    sha256: None,
                }],
            })],
            docs: vec![TenantDocDescriptor::Simple(SimpleTenantDocEntry {
                url: doc_url.to_string(),
                file_name: "greentic-fast2flow-guide.md".to_string(),
            })],
        })?;
        let installer = Installer::new(
            FakeTenantManifestSource { manifest },
            FakeDownloader {
                responses: HashMap::from([
                    (tool_url.to_string(), tool_archive),
                    (doc_url.to_string(), b"# fast2flow\n".to_vec()),
                ]),
            },
            test_env(&temp)?,
        );
        installer.install_tenant("3point", "secret-token")?;
        assert_eq!(
            fs::read(temp.path().join("bin/greentic-fast2flow"))?,
            b"bin"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("docs/greentic-fast2flow-guide.md"))?,
            "# fast2flow\n"
        );
        Ok(())
    }

    #[test]
    fn expected_binary_name_strips_release_target_and_version() {
        let name = expected_binary_name(
            "greentic-fast2flow",
            "https://github.com/greentic-biz/greentic-fast2flow/releases/download/v0.4.1/greentic-fast2flow-v0.4.1-x86_64-unknown-linux-gnu.tar.gz",
        );
        assert_eq!(name, "greentic-fast2flow");
    }

    #[test]
    fn extracts_tar_gz_binary_with_versioned_entry_name() -> Result<()> {
        let temp = TempDir::new()?;
        let archive = tar_gz_with_binary(
            "greentic-mcp-generator-0.4.14-x86_64-unknown-linux-gnu",
            b"bin",
        );
        let out = extract_tar_gz_binary(&archive, "greentic-mcp-generator", temp.path())?;
        assert_eq!(
            out,
            temp.path()
                .join("greentic-mcp-generator-0.4.14-x86_64-unknown-linux-gnu")
        );
        assert_eq!(fs::read(out)?, b"bin");
        Ok(())
    }

    #[test]
    fn extracts_tar_gz_binary_even_when_archive_name_differs() -> Result<()> {
        let temp = TempDir::new()?;
        let archive = tar_gz_with_binary("greentic-mcp-gen", b"bin");
        let out = extract_tar_gz_binary(&archive, "greentic-mcp-generator", temp.path())?;
        assert_eq!(out, temp.path().join("greentic-mcp-gen"));
        assert_eq!(fs::read(out)?, b"bin");
        Ok(())
    }
}
