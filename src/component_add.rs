use std::convert::TryFrom;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use greentic_distributor_client::{
    DistributorClient, DistributorClientConfig, DistributorEnvironmentId, HttpDistributorClient,
    ResolveComponentRequest,
};
use greentic_pack::builder::ComponentEntry;
use greentic_types::{EnvId, TenantCtx, TenantId};
use reqwest::blocking::Client;
use semver::Version;
use serde_json::json;
use tokio::runtime::Runtime;

use crate::config;
use crate::distributor;
use crate::pack_init::{
    PackInitIntent, WorkspaceComponent, WorkspaceManifest, manifest_path, slugify,
};

#[derive(Debug, Clone)]
struct SimpleStub {
    artifact_path: PathBuf,
    digest: String,
    signature: greentic_types::distributor::SignatureSummary,
    cache: greentic_types::distributor::CacheInfo,
}

pub fn run_component_add(
    coordinate: &str,
    profile: Option<&str>,
    intent: PackInitIntent,
) -> Result<PathBuf> {
    let coordinate_path = PathBuf::from(coordinate);
    if coordinate_path.exists() {
        return Ok(coordinate_path);
    }

    let offline = std::env::var("GREENTIC_DEV_OFFLINE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let stubbed_response = load_stubbed_response();
    if offline && stubbed_response.is_none() {
        bail!(
            "offline mode enabled (GREENTIC_DEV_OFFLINE=1) and coordinate `{coordinate}` is not a local path; provide a stub via GREENTIC_DEV_RESOLVE_STUB or use a local component path"
        );
    }

    let (component_id, version_req) = parse_coordinate(coordinate)?;

    let response = if let Some(resp) = stubbed_response {
        resp?
    } else {
        let cfg = config::load_with_meta(None)?;
        let profile = distributor::resolve_profile(&cfg, profile)?;
        let tenant_ctx = build_tenant_ctx(&profile)?;
        let environment_id = DistributorEnvironmentId::from(profile.environment_id.as_str());
        let pack_id = detect_pack_id().unwrap_or_else(|| "greentic-dev-local".to_string());

        let req = ResolveComponentRequest {
            tenant: tenant_ctx.clone(),
            environment_id,
            pack_id,
            component_id: component_id.clone(),
            version: version_req.to_string(),
            extra: json!({ "intent": format!("{:?}", intent) }),
        };

        let client = http_client(&profile)?;
        let rt = Runtime::new().context("failed to start tokio runtime for distributor client")?;
        rt.block_on(client.resolve_component(req))?
    };

    let artifact_bytes = fetch_artifact(&response.artifact)?;
    let (cache_dir, cache_path) =
        write_component_to_cache(&component_id, &version_req, &artifact_bytes)?;
    update_manifest(coordinate, &component_id, &version_req, &cache_path)?;

    println!(
        "Resolved {} -> {}@{}",
        coordinate, component_id, version_req
    );
    println!("Cached component at {}", cache_path.display());
    println!(
        "Updated workspace manifest at {}",
        manifest_path()?.display()
    );

    Ok(cache_dir)
}

fn parse_coordinate(input: &str) -> Result<(String, String)> {
    if let Some((id, ver)) = input.rsplit_once('@') {
        Ok((id.to_string(), ver.to_string()))
    } else {
        Ok((input.to_string(), "*".to_string()))
    }
}

fn build_tenant_ctx(profile: &distributor::DistributorProfile) -> Result<TenantCtx> {
    let env = EnvId::from_str(&profile.environment_id)
        .or_else(|_| EnvId::try_from(profile.environment_id.as_str()))
        .map_err(|err| anyhow!("invalid environment id `{}`: {err}", profile.environment_id))?;
    let tenant = TenantId::from_str(&profile.tenant_id)
        .or_else(|_| TenantId::try_from(profile.tenant_id.as_str()))
        .map_err(|err| anyhow!("invalid tenant id `{}`: {err}", profile.tenant_id))?;
    Ok(TenantCtx::new(env, tenant))
}

fn http_client(profile: &distributor::DistributorProfile) -> Result<HttpDistributorClient> {
    let env_id = EnvId::from_str(&profile.environment_id)
        .or_else(|_| EnvId::try_from(profile.environment_id.as_str()))
        .map_err(|err| anyhow!("invalid environment id `{}`: {err}", profile.environment_id))?;
    let tenant_id = TenantId::from_str(&profile.tenant_id)
        .or_else(|_| TenantId::try_from(profile.tenant_id.as_str()))
        .map_err(|err| anyhow!("invalid tenant id `{}`: {err}", profile.tenant_id))?;
    let cfg = DistributorClientConfig {
        base_url: Some(profile.url.clone()),
        environment_id: DistributorEnvironmentId::from(profile.environment_id.as_str()),
        tenant: TenantCtx::new(env_id, tenant_id),
        auth_token: profile.token.clone(),
        extra_headers: profile.headers.clone(),
        request_timeout: None,
    };
    HttpDistributorClient::new(cfg).map_err(Into::into)
}

fn fetch_artifact(location: &greentic_distributor_client::ArtifactLocation) -> Result<Bytes> {
    match location {
        greentic_distributor_client::ArtifactLocation::FilePath { path } => {
            if path.starts_with("http://") || path.starts_with("https://") {
                let client = Client::new();
                let resp = client
                    .get(path)
                    .send()
                    .context("failed to download artifact")?;
                if !resp.status().is_success() {
                    bail!("artifact download failed with status {}", resp.status());
                }
                resp.bytes().map_err(Into::into)
            } else if let Some(rest) = path.strip_prefix("file://") {
                fs::read(rest)
                    .map(Bytes::from)
                    .with_context(|| format!("failed to read component at {}", rest))
            } else {
                fs::read(path)
                    .map(Bytes::from)
                    .with_context(|| format!("failed to read component at {}", path))
            }
        }
        greentic_distributor_client::ArtifactLocation::OciReference { reference } => {
            bail!("OCI component artifacts are not supported yet ({reference})")
        }
        greentic_distributor_client::ArtifactLocation::DistributorInternal { handle } => {
            bail!("Distributor internal artifacts are not supported yet ({handle})")
        }
    }
}

fn write_component_to_cache(
    component_id: &str,
    version: &str,
    bytes: &Bytes,
) -> Result<(PathBuf, PathBuf)> {
    let mut path = cache_base_dir()?;
    let slug = cache_slug_parts(component_id, version);
    path.push(slug);
    fs::create_dir_all(&path).with_context(|| format!("failed to create {}", path.display()))?;
    let file_path = path.join("artifact.wasm");
    fs::write(&file_path, bytes)
        .with_context(|| format!("failed to write {}", file_path.display()))?;
    Ok((path, file_path))
}

fn cache_base_dir() -> Result<PathBuf> {
    let mut base = std::env::current_dir().context("unable to determine workspace root")?;
    base.push(".greentic");
    base.push("components");
    fs::create_dir_all(&base)
        .with_context(|| format!("failed to create cache directory {}", base.display()))?;
    Ok(base)
}

fn cache_slug_parts(component_id: &str, version: &str) -> String {
    slugify(&format!("{}-{}", component_id.replace('/', "-"), version))
}

fn update_manifest(
    coordinate: &str,
    component_id: &str,
    version: &str,
    wasm_path: &Path,
) -> Result<()> {
    let manifest_path = manifest_path()?;
    let mut manifest: WorkspaceManifest = if manifest_path.exists() {
        let data = fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        serde_json::from_str(&data)
            .with_context(|| format!("failed to parse {}", manifest_path.display()))?
    } else {
        WorkspaceManifest::default()
    };

    let entry = WorkspaceComponent {
        coordinate: coordinate.to_string(),
        entry: ComponentEntry {
            name: component_id.to_string(),
            version: Version::parse(version).unwrap_or_else(|_| Version::new(0, 0, 0)),
            file_wasm: wasm_path.display().to_string(),
            hash_blake3: String::new(),
            schema_file: None,
            manifest_file: None,
            world: None,
            capabilities: None,
        },
    };

    let mut replaced = false;
    for existing in manifest.components.iter_mut() {
        if existing.entry.name == entry.entry.name {
            *existing = entry.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        manifest.components.push(entry);
    }

    let rendered =
        serde_json::to_string_pretty(&manifest).context("failed to render workspace manifest")?;
    fs::write(&manifest_path, rendered)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    Ok(())
}

fn detect_pack_id() -> Option<String> {
    let candidates = ["pack.toml", "Pack.toml"];
    for candidate in candidates {
        let path = Path::new(candidate);
        if path.exists() {
            let data = fs::read_to_string(path).ok()?;
            if let Ok(value) = data.parse::<toml::Value>()
                && let Some(id) = value.get("pack_id").and_then(|v| v.as_str())
            {
                return Some(id.to_string());
            }
        }
    }
    None
}

fn load_stubbed_response() -> Option<Result<greentic_distributor_client::ResolveComponentResponse>>
{
    let path = std::env::var("GREENTIC_DEV_RESOLVE_STUB").ok()?;
    let data = fs::read_to_string(&path)
        .map_err(|err| anyhow!("failed to read stub response {}: {err}", path))
        .ok()?;

    // First try to parse the real response shape.
    if let Ok(resp) =
        serde_json::from_str::<greentic_distributor_client::ResolveComponentResponse>(&data)
    {
        return Some(Ok(resp));
    }

    // Fallback: accept a minimal stub JSON.
    let parsed = serde_json::from_str::<serde_json::Value>(&data).ok()?;
    let Some(artifact_path) = parsed
        .get("artifact_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
    else {
        return Some(Err(anyhow!(
            "stub missing `artifact_path` (expected JSON with artifact_path pointing to the component wasm)"
        )));
    };
    let digest = parsed
        .get("digest")
        .and_then(|v| v.as_str())
        .unwrap_or("sha256:stub")
        .to_string();
    let signature = greentic_types::distributor::SignatureSummary {
        verified: false,
        signer: "stub".to_string(),
        extra: serde_json::Value::Object(serde_json::Map::new()),
    };
    let cache = greentic_types::distributor::CacheInfo {
        size_bytes: 0,
        last_used_utc: "stub".to_string(),
        last_refreshed_utc: "stub".to_string(),
    };
    let stub = SimpleStub {
        artifact_path,
        digest,
        signature,
        cache,
    };

    Some(Ok(to_resolve_response(stub)))
}

fn to_resolve_response(stub: SimpleStub) -> greentic_distributor_client::ResolveComponentResponse {
    greentic_distributor_client::ResolveComponentResponse {
        status: greentic_types::distributor::ComponentStatus::Ready,
        digest: greentic_types::distributor::ComponentDigest(stub.digest),
        artifact: greentic_distributor_client::ArtifactLocation::FilePath {
            path: stub.artifact_path.display().to_string(),
        },
        signature: stub.signature,
        cache: stub.cache,
        secret_requirements: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{cache_slug_parts, parse_coordinate};

    #[test]
    fn parse_coordinate_defaults_to_wildcard_version() {
        let (name, version) = parse_coordinate("demo.component").unwrap();
        assert_eq!(name, "demo.component");
        assert_eq!(version, "*");
    }

    #[test]
    fn parse_coordinate_splits_explicit_version() {
        let (name, version) = parse_coordinate("demo.component@1.2.3").unwrap();
        assert_eq!(name, "demo.component");
        assert_eq!(version, "1.2.3");
    }

    #[test]
    fn cache_slug_normalizes_component_and_version() {
        let slug = cache_slug_parts("org/demo.component", "1.2.3");
        assert!(slug.contains("org-demo-component"));
        assert!(slug.contains("1-2-3"));
    }
}
