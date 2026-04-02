use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use convert_case::{Case, Casing};
use once_cell::sync::Lazy;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use wit_component::{DecodedWasm, decode as decode_component};
use wit_parser::{Resolve, WorldId, WorldItem};

use crate::path_safety::normalize_under_root;

static WORKSPACE_ROOT: Lazy<PathBuf> = Lazy::new(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));

const TEMPLATE_COMPONENT_CARGO: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/templates/component/Cargo.toml.in"
));
const TEMPLATE_SRC_LIB: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/templates/component/src/lib.rs"
));
const TEMPLATE_PROVIDER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/templates/component/provider.toml"
));
const TEMPLATE_SCHEMA_CONFIG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/templates/component/schemas/v1/config.schema.json"
));
const TEMPLATE_README: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/templates/component/README.md"
));

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderMetadata {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    abi: AbiSection,
    capabilities: CapabilitiesSection,
    exports: ExportsSection,
    #[serde(default)]
    imports: ImportsSection,
    artifact: ArtifactSection,
    #[serde(default)]
    docs: Option<DocsSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AbiSection {
    interfaces_version: String,
    types_version: String,
    component_runtime: String,
    world: String,
    #[serde(default)]
    wit_packages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CapabilitiesSection {
    #[serde(default)]
    secrets: bool,
    #[serde(default)]
    telemetry: bool,
    #[serde(default)]
    network: bool,
    #[serde(default)]
    filesystem: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExportsSection {
    #[serde(default)]
    provides: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ImportsSection {
    #[serde(default)]
    requires: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactSection {
    format: String,
    path: String,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    created: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DocsSection {
    #[serde(default)]
    readme: Option<String>,
    #[serde(default)]
    schemas: Vec<String>,
}

#[derive(Debug)]
struct ValidationReport {
    provider: ProviderMetadata,
    component_dir: PathBuf,
    artifact_path: PathBuf,
    sha256: String,
    world: String,
    packages: Vec<String>,
}

#[derive(Debug, Clone)]
struct Versions {
    interfaces: String,
    types: String,
    component_runtime: String,
    component_wit_version: String,
    secrets_wit_version: String,
    state_wit_version: String,
    http_wit_version: String,
    telemetry_wit_version: String,
}

impl Versions {
    fn load() -> Result<Self> {
        let interfaces_version = resolved_version("greentic-interfaces")?;
        let types_version = resolved_version("greentic-types")?;
        let component_runtime_version = resolved_version("greentic-component")?;

        let interfaces_root = find_crate_source("greentic-interfaces", &interfaces_version)?;
        let component_wit_version = detect_component_node_world_version(&interfaces_root)?;
        let secrets_wit_version = detect_wit_package_version(&interfaces_root, "secrets")?;
        let state_wit_version = detect_wit_package_version(&interfaces_root, "state")?;
        let http_wit_version = detect_wit_package_version(&interfaces_root, "http")?;
        let telemetry_wit_version = detect_wit_package_version(&interfaces_root, "telemetry")?;

        Ok(Self {
            interfaces: interfaces_version,
            types: types_version,
            component_runtime: component_runtime_version,
            component_wit_version,
            secrets_wit_version,
            state_wit_version,
            http_wit_version,
            telemetry_wit_version,
        })
    }
}

static VERSIONS: Lazy<Versions> =
    Lazy::new(|| Versions::load().expect("load greentic crate versions"));

pub fn run_component_command(command: ComponentCommands) -> Result<()> {
    match command {
        ComponentCommands::New(args) => new_component(args),
        ComponentCommands::Validate(args) => validate_command(args),
        ComponentCommands::Pack(args) => pack_command(args),
    }
}

#[derive(Subcommand, Debug, Clone)]
pub enum ComponentCommands {
    /// Scaffold a new component repository
    New(NewComponentArgs),
    /// Build and validate a component against pinned interfaces
    Validate(ValidateArgs),
    /// Package a component into `packs/<name>/<version>`
    Pack(PackArgs),
}

#[derive(Args, Debug, Clone)]
pub struct NewComponentArgs {
    /// Name of the component (kebab-case recommended)
    name: String,
    /// Optional directory where the component should be created
    #[arg(long, value_name = "DIR")]
    dir: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct ValidateArgs {
    /// Path to the component directory
    #[arg(long, value_name = "PATH", default_value = ".")]
    path: PathBuf,
    /// Skip cargo component build (use the existing artifact)
    #[arg(long)]
    skip_build: bool,
}

#[derive(Args, Debug, Clone)]
pub struct PackArgs {
    /// Path to the component directory
    #[arg(long, value_name = "PATH", default_value = ".")]
    path: PathBuf,
    /// Output directory for generated packs (defaults to `<component>/packs`)
    #[arg(long, value_name = "DIR")]
    out_dir: Option<PathBuf>,
    /// Skip cargo component build before packing
    #[arg(long)]
    skip_build: bool,
}

pub fn new_component(args: NewComponentArgs) -> Result<()> {
    let context = TemplateContext::new(&args.name)?;
    let workspace_root = workspace_root()?;
    let base_dir = match args.dir {
        Some(dir) => normalize_under_root(&workspace_root, &dir)?,
        None => workspace_root.clone(),
    };
    fs::create_dir_all(&base_dir)
        .with_context(|| format!("failed to prepare base directory {}", base_dir.display()))?;
    let component_dir = base_dir.join(context.component_dir());

    if component_dir.exists() {
        bail!(
            "component directory `{}` already exists",
            component_dir.display()
        );
    }

    println!(
        "Creating new component scaffold at `{}`",
        component_dir.display()
    );

    create_dir(component_dir.join("src"))?;
    create_dir(component_dir.join("schemas/v1"))?;

    write_template(
        &component_dir.join("Cargo.toml"),
        TEMPLATE_COMPONENT_CARGO,
        &context,
    )?;
    write_template(&component_dir.join("README.md"), TEMPLATE_README, &context)?;
    write_template(
        &component_dir.join("provider.toml"),
        TEMPLATE_PROVIDER,
        &context,
    )?;
    write_template(
        &component_dir.join("src/lib.rs"),
        TEMPLATE_SRC_LIB,
        &context,
    )?;
    write_template(
        &component_dir.join("schemas/v1/config.schema.json"),
        TEMPLATE_SCHEMA_CONFIG,
        &context,
    )?;

    println!(
        "Component `{}` scaffolded successfully.",
        context.component_name
    );

    Ok(())
}

pub fn validate_command(args: ValidateArgs) -> Result<()> {
    let workspace_root = workspace_root()?;
    let report = validate_component(&workspace_root, &args.path, !args.skip_build)?;
    print_validation_summary(&report);
    Ok(())
}

pub fn pack_command(args: PackArgs) -> Result<()> {
    let workspace_root = workspace_root()?;
    let report = validate_component(&workspace_root, &args.path, !args.skip_build)?;
    let base_out = match args.out_dir {
        Some(ref dir) if dir.is_absolute() => {
            bail!("--out-dir must be relative to the component directory")
        }
        Some(ref dir)
            if dir
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir)) =>
        {
            bail!("--out-dir must not contain parent segments (`..`)")
        }
        Some(ref dir) => report.component_dir.join(dir),
        None => report.component_dir.join("packs"),
    };
    fs::create_dir_all(&base_out)
        .with_context(|| format!("failed to create {}", base_out.display()))?;

    let dest_dir = base_out
        .join(&report.provider.name)
        .join(&report.provider.version);
    if dest_dir.exists() {
        fs::remove_dir_all(&dest_dir)
            .with_context(|| format!("failed to clear {}", dest_dir.display()))?;
    }
    fs::create_dir_all(&dest_dir)
        .with_context(|| format!("failed to create {}", dest_dir.display()))?;

    let artifact_file = format!("{}-{}.wasm", report.provider.name, report.provider.version);
    let dest_wasm = dest_dir.join(&artifact_file);
    fs::copy(&report.artifact_path, &dest_wasm).with_context(|| {
        format!(
            "failed to copy {} to {}",
            report.artifact_path.display(),
            dest_wasm.display()
        )
    })?;

    let mut meta = report.provider.clone();
    meta.artifact.path = artifact_file.clone();
    meta.artifact.sha256 = report.sha256.clone();
    meta.artifact.created = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("unable to format timestamp")?;
    meta.abi.wit_packages = report.packages.clone();

    let meta_path = dest_dir.join("meta.json");
    let meta_file = fs::File::create(&meta_path)
        .with_context(|| format!("failed to create {}", meta_path.display()))?;
    serde_json::to_writer_pretty(meta_file, &meta)
        .with_context(|| format!("failed to write {}", meta_path.display()))?;

    let mut sums =
        fs::File::create(dest_dir.join("SHA256SUMS")).context("failed to create SHA256SUMS")?;
    writeln!(sums, "{}  {}", report.sha256, artifact_file).context("failed to write SHA256SUMS")?;

    println!("✓ Packed component at {}", dest_dir.display());
    Ok(())
}

fn create_dir(path: PathBuf) -> Result<()> {
    fs::create_dir_all(&path)
        .with_context(|| format!("failed to create directory `{}`", path.display()))
}

fn write_template(path: &Path, template: &str, context: &TemplateContext) -> Result<()> {
    if path.exists() {
        bail!("file `{}` already exists", path.display());
    }

    let rendered = render_template(template, context);
    fs::write(path, rendered).with_context(|| format!("failed to write `{}`", path.display()))
}

fn render_template(template: &str, context: &TemplateContext) -> String {
    let mut output = template.to_owned();
    for (key, value) in &context.placeholders {
        let token = format!("{{{{{key}}}}}");
        output = output.replace(&token, value);
    }
    output
}

fn detect_component_node_world_version(crate_root: &Path) -> Result<String> {
    let wit_dir = crate_root.join("wit");
    let namespace_dir = wit_dir.join("greentic");
    let prefix = "component@";
    let mut best: Option<(Version, PathBuf)> = None;

    for entry in fs::read_dir(&namespace_dir).with_context(|| {
        format!(
            "failed to read namespace directory {}",
            namespace_dir.display()
        )
    })? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("non-unicode filename under {}", namespace_dir.display()))?;
        if let Some(rest) = name.strip_prefix(prefix) {
            let version = Version::parse(rest)
                .with_context(|| format!("invalid semver `{rest}` for {prefix}"))?;
            let package_path = path.join("package.wit");
            let contents = fs::read_to_string(&package_path).with_context(|| {
                format!("failed to read package file {}", package_path.display())
            })?;
            if contents.contains("export node") {
                match &best {
                    Some((best_ver, _)) if version <= *best_ver => {}
                    _ => best = Some((version, path)),
                }
            }
        }
    }

    if let Some((version, _)) = best {
        return Ok(version.to_string());
    }

    detect_wit_package_version(crate_root, "component")
}

fn detect_wit_package_version(crate_root: &Path, prefix: &str) -> Result<String> {
    let wit_dir = crate_root.join("wit");
    let namespace_dir = wit_dir.join("greentic");
    let prefix = format!("{prefix}@");

    let mut best: Option<(Version, PathBuf)> = None;
    for entry in fs::read_dir(&namespace_dir).with_context(|| {
        format!(
            "failed to read namespace directory {}",
            namespace_dir.display()
        )
    })? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("non-unicode filename under {}", namespace_dir.display()))?;
        if let Some(rest) = name.strip_prefix(&prefix) {
            let version = Version::parse(rest)
                .with_context(|| format!("invalid semver `{rest}` for {prefix}"))?;
            if best.as_ref().is_none_or(|(current, _)| &version > current) {
                best = Some((version, path));
            }
        }
    }

    match best {
        Some((version, _)) => Ok(version.to_string()),
        None => Err(anyhow!(
            "unable to locate WIT package `{}` under {}",
            prefix,
            namespace_dir.display()
        )),
    }
}

#[derive(Deserialize)]
struct LockPackage {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct LockFile {
    package: Vec<LockPackage>,
}

fn resolved_version(crate_name: &str) -> Result<String> {
    let lock_path = WORKSPACE_ROOT.join("Cargo.lock");
    let contents = fs::read_to_string(&lock_path)
        .with_context(|| format!("failed to read {}", lock_path.display()))?;
    let lock: LockFile =
        toml::from_str(&contents).with_context(|| format!("invalid {}", lock_path.display()))?;

    let mut best: Option<(Version, String)> = None;
    for pkg in lock
        .package
        .into_iter()
        .filter(|pkg| pkg.name == crate_name)
    {
        let version = Version::parse(&pkg.version)
            .with_context(|| format!("invalid semver `{}` for {}", pkg.version, crate_name))?;
        if best.as_ref().is_none_or(|(current, _)| &version > current) {
            best = Some((version, pkg.version));
        }
    }

    match best {
        Some((_, version)) => Ok(version),
        None => Err(anyhow!(
            "crate `{}` not found in {}",
            crate_name,
            lock_path.display()
        )),
    }
}

fn cargo_home() -> Result<PathBuf> {
    if let Ok(path) = env::var("CARGO_HOME") {
        return Ok(PathBuf::from(path));
    }
    if let Ok(home) = env::var("HOME") {
        return Ok(PathBuf::from(home).join(".cargo"));
    }
    Err(anyhow!(
        "unable to determine CARGO_HOME; set the environment variable explicitly"
    ))
}

fn find_crate_source(crate_name: &str, version: &str) -> Result<PathBuf> {
    let home = cargo_home()?;
    let registry_src = home.join("registry/src");
    if !registry_src.exists() {
        return Err(anyhow!(
            "cargo registry src directory not found at {}",
            registry_src.display()
        ));
    }

    for index in fs::read_dir(&registry_src)? {
        let index_path = index?.path();
        if !index_path.is_dir() {
            continue;
        }
        let candidate = index_path.join(format!("{crate_name}-{version}"));
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(anyhow!(
        "crate `{}` version `{}` not found under {}",
        crate_name,
        version,
        registry_src.display()
    ))
}

struct TemplateContext {
    component_name: String,
    component_kebab: String,
    placeholders: HashMap<String, String>,
}

impl TemplateContext {
    fn new(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            bail!("component name cannot be empty");
        }

        let component_kebab = trimmed.to_case(Case::Kebab);
        let component_snake = trimmed.to_case(Case::Snake);
        let component_pascal = trimmed.to_case(Case::Pascal);
        let component_name = component_kebab.clone();
        let versions = VERSIONS.clone();

        let mut placeholders = HashMap::new();
        placeholders.insert("component_name".into(), component_name.clone());
        placeholders.insert("component_kebab".into(), component_kebab.clone());
        placeholders.insert("component_snake".into(), component_snake.clone());
        placeholders.insert("component_pascal".into(), component_pascal.clone());
        placeholders.insert("component_crate".into(), component_kebab.clone());
        placeholders.insert(
            "component_dir".into(),
            format!("component-{component_kebab}"),
        );
        placeholders.insert("interfaces_version".into(), versions.interfaces.clone());
        placeholders.insert("types_version".into(), versions.types.clone());
        placeholders.insert(
            "component_runtime_version".into(),
            versions.component_runtime.clone(),
        );
        placeholders.insert(
            "component_world_version".into(),
            versions.component_wit_version.clone(),
        );
        placeholders.insert(
            "interfaces_guest_version".into(),
            versions.interfaces.clone(),
        );
        placeholders.insert(
            "secrets_wit_version".into(),
            versions.secrets_wit_version.clone(),
        );
        placeholders.insert(
            "state_wit_version".into(),
            versions.state_wit_version.clone(),
        );
        placeholders.insert("http_wit_version".into(), versions.http_wit_version.clone());
        placeholders.insert(
            "telemetry_wit_version".into(),
            versions.telemetry_wit_version.clone(),
        );

        Ok(Self {
            component_name,
            component_kebab,
            placeholders,
        })
    }

    fn component_dir(&self) -> String {
        format!("component-{}", self.component_kebab)
    }
}

fn print_validation_summary(report: &ValidationReport) {
    println!(
        "✓ Validated {} {}",
        report.provider.name, report.provider.version
    );
    println!("  artifact: {}", report.artifact_path.display());
    println!("  sha256 : {}", report.sha256);
    println!("  world  : {}", report.world);
    println!("  packages:");
    for pkg in &report.packages {
        println!("    - {pkg}");
    }
}

fn validate_component(workspace_root: &Path, path: &Path, build: bool) -> Result<ValidationReport> {
    let component_dir = normalize_under_root(workspace_root, path)?;

    if build {
        ensure_cargo_component_installed()?;
        run_cargo_component_build(&component_dir)?;
    }

    let provider_path = normalize_under_root(&component_dir, Path::new("provider.toml"))?;
    let provider = load_provider(&provider_path)?;

    let versions = Versions::load()?;
    ensure_version_alignment(&provider, &versions)?;

    let mut attempted = Vec::new();
    let mut artifact_path: Option<PathBuf> = None;
    for candidate in candidate_artifact_paths(&provider.artifact.path) {
        let resolved = resolve_path(&component_dir, Path::new(&candidate));
        attempted.push(resolved.clone());
        if resolved.exists() {
            artifact_path = Some(resolved);
            break;
        }
    }
    let artifact_path = match artifact_path {
        Some(path) => normalize_under_root(&component_dir, &path).with_context(|| {
            format!(
                "artifact path escapes component directory {}",
                component_dir.display()
            )
        })?,
        None => {
            let paths = attempted
                .into_iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("artifact path not found; checked {paths}");
        }
    };

    let wasm_bytes = fs::read(&artifact_path)
        .with_context(|| format!("failed to read {}", artifact_path.display()))?;
    let sha256 = sha256_hex(&wasm_bytes);

    let decoded = decode_component(&wasm_bytes).context("failed to decode component")?;
    let (resolve, world_id) = match decoded {
        DecodedWasm::Component(resolve, world) => (resolve, world),
        DecodedWasm::WitPackage(_, _) => {
            bail!("expected a component artifact but found a WIT package bundle")
        }
    };
    let (packages, world, export_package) = extract_wit_metadata(&resolve, world_id)?;

    if packages.is_empty() {
        bail!("no WIT packages embedded in component artifact");
    }

    if provider.abi.world != world {
        if let Some(expected_pkg) = world_to_package_id(&provider.abi.world) {
            if let Some(actual_pkg) = export_package {
                if actual_pkg != expected_pkg {
                    bail!(
                        "provider world `{}` expects package '{}', but embedded exports use '{}'",
                        provider.abi.world,
                        expected_pkg,
                        actual_pkg
                    );
                }
            } else if !packages.iter().any(|pkg| pkg == &expected_pkg) {
                bail!(
                    "provider world `{}` expects package '{}', which was not embedded (found {:?})",
                    provider.abi.world,
                    expected_pkg,
                    packages
                );
            }
        } else {
            bail!(
                "provider world `{}` is not formatted as <namespace>:<package>/<world>@<version>",
                provider.abi.world
            );
        }
    }

    let expected_packages: BTreeSet<_> = provider.abi.wit_packages.iter().cloned().collect();
    if !expected_packages.is_empty() {
        let actual_greentic: BTreeSet<_> = packages
            .iter()
            .filter(|pkg| pkg.starts_with("greentic:"))
            .cloned()
            .collect();
        if !expected_packages.is_subset(&actual_greentic) {
            bail!(
                "provider wit_packages {expected_packages:?} not satisfied by embedded packages \
                 {actual_greentic:?}"
            );
        }
    }

    Ok(ValidationReport {
        provider,
        component_dir,
        artifact_path,
        sha256,
        world,
        packages,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn resolve_path(base: &Path, raw: impl AsRef<Path>) -> PathBuf {
    let raw_path = raw.as_ref();
    if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        base.join(raw_path)
    }
}

fn candidate_artifact_paths(original: &str) -> Vec<String> {
    let mut paths = Vec::new();
    paths.push(original.to_string());

    for (from, to) in [
        ("wasm32-wasip2", "wasm32-wasip1"),
        ("wasm32-wasip2", "wasm32-wasi"),
        ("wasm32-wasip1", "wasm32-wasip2"),
        ("wasm32-wasip1", "wasm32-wasi"),
        ("wasm32-wasi", "wasm32-wasip2"),
        ("wasm32-wasi", "wasm32-wasip1"),
    ] {
        if original.contains(from) {
            let candidate = original.replace(from, to);
            if candidate != original && !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }

    paths
}

fn ensure_cargo_component_installed() -> Result<()> {
    let status = Command::new("cargo")
        .arg("component")
        .arg("--version")
        .status();
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => bail!(
            "cargo-component is required. Install with `cargo install cargo-component --locked`."
        ),
        Err(err) => Err(anyhow!(
            "failed to execute `cargo component --version`: {err}. Install cargo-component with `cargo install cargo-component --locked`."
        )),
    }
}

fn run_cargo_component_build(component_dir: &Path) -> Result<()> {
    let cache_dir = component_dir.join("target").join(".component-cache");
    let status = Command::new("cargo")
        .current_dir(component_dir)
        .arg("component")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg("wasm32-wasip2")
        .env("CARGO_COMPONENT_CACHE_DIR", cache_dir.as_os_str())
        .env("CARGO_NET_OFFLINE", "true")
        .status()
        .with_context(|| {
            format!(
                "failed to run `cargo component build` in {}",
                component_dir.display()
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        bail!("cargo component build failed")
    }
}

fn load_provider(path: &Path) -> Result<ProviderMetadata> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read provider metadata {}", path.display()))?;
    let provider: ProviderMetadata =
        toml::from_str(&contents).context("provider.toml is not valid TOML")?;
    if provider.artifact.format != "wasm-component" {
        bail!(
            "artifact.format must be `wasm-component`, found `{}`",
            provider.artifact.format
        );
    }
    Ok(provider)
}

fn ensure_version_alignment(provider: &ProviderMetadata, versions: &Versions) -> Result<()> {
    if provider.abi.interfaces_version != versions.interfaces {
        bail!(
            "provider abi.interfaces_version `{}` does not match pinned `{}`",
            provider.abi.interfaces_version,
            versions.interfaces
        );
    }
    if provider.abi.types_version != versions.types {
        bail!(
            "provider abi.types_version `{}` does not match pinned `{}`",
            provider.abi.types_version,
            versions.types
        );
    }
    Ok(())
}

fn extract_wit_metadata(
    resolve: &Resolve,
    world_id: WorldId,
) -> Result<(Vec<String>, String, Option<String>)> {
    let mut packages = Vec::new();
    for (_, package) in resolve.packages.iter() {
        let name = &package.name;
        if name.namespace == "root" {
            continue;
        }
        if let Some(version) = &name.version {
            packages.push(format!("{}:{}@{}", name.namespace, name.name, version));
        } else {
            packages.push(format!("{}:{}", name.namespace, name.name));
        }
    }
    packages.sort();
    packages.dedup();

    let world = &resolve.worlds[world_id];
    let mut export_package = None;
    for item in world.exports.values() {
        if let WorldItem::Interface { id, .. } = item {
            let iface = &resolve.interfaces[*id];
            if let Some(pkg_id) = iface.package {
                let pkg = &resolve.packages[pkg_id].name;
                if pkg.namespace != "root" {
                    let mut ident = format!("{}:{}", pkg.namespace, pkg.name);
                    if let Some(version) = &pkg.version {
                        ident.push('@');
                        ident.push_str(&version.to_string());
                    }
                    export_package.get_or_insert(ident);
                }
            }
        }
    }

    let world_string = if let Some(pkg_id) = world.package {
        let pkg = &resolve.packages[pkg_id];
        if let Some(version) = &pkg.name.version {
            format!(
                "{}:{}/{}@{}",
                pkg.name.namespace, pkg.name.name, world.name, version
            )
        } else {
            format!("{}:{}/{}", pkg.name.namespace, pkg.name.name, world.name)
        }
    } else {
        world.name.clone()
    };

    Ok((packages, world_string, export_package))
}

fn world_to_package_id(world: &str) -> Option<String> {
    let (pkg_part, rest) = world.split_once('/')?;
    let (_, version) = rest.rsplit_once('@')?;
    Some(format!("{pkg_part}@{version}"))
}

fn workspace_root() -> Result<PathBuf> {
    env::current_dir()
        .context("unable to determine current directory")?
        .canonicalize()
        .context("failed to canonicalize workspace root")
}
