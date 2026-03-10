//! Bundle lifecycle management commands.
//!
//! Provides CLI commands for managing Greentic bundles using greentic-setup library:
//! - `bundle add` - Add a pack to a bundle
//! - `bundle setup` - Run setup flow for a provider
//! - `bundle update` - Update a provider's configuration
//! - `bundle remove` - Remove a provider from a bundle
//! - `bundle status` - Show bundle status

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use greentic_setup::{SetupEngine, SetupMode};
use greentic_setup::engine::{SetupConfig, SetupRequest};
use greentic_setup::plan::TenantSelection;

use crate::cli::{BundleAddArgs, BundleRemoveArgs, BundleSetupArgs, BundleStatusArgs};

/// Run the `bundle add` command.
pub fn add(args: BundleAddArgs) -> Result<()> {
    let bundle_dir = resolve_bundle_dir(args.bundle)?;

    println!("Adding pack to bundle...");
    println!("  Pack ref: {}", args.pack_ref);
    println!("  Bundle: {}", bundle_dir.display());
    println!("  Tenant: {}", args.tenant);
    println!("  Team: {}", args.team.as_deref().unwrap_or("default"));
    println!("  Env: {}", args.env);

    if args.dry_run {
        println!("\n[dry-run] Would add pack to bundle");
        return Ok(());
    }

    // Create bundle structure if it doesn't exist
    if !bundle_dir.join("greentic.demo.yaml").exists() {
        greentic_setup::bundle::create_demo_bundle_structure(&bundle_dir, None)
            .context("failed to create bundle structure")?;
        println!("Created bundle structure at {}", bundle_dir.display());
    }

    // Build setup request
    let request = SetupRequest {
        bundle: bundle_dir.clone(),
        pack_refs: vec![args.pack_ref.clone()],
        tenants: vec![TenantSelection {
            tenant: args.tenant.clone(),
            team: args.team.clone(),
            allow_paths: Vec::new(),
        }],
        ..Default::default()
    };

    // Create engine and build plan
    let config = SetupConfig {
        tenant: args.tenant,
        team: args.team,
        env: args.env,
        offline: false,
        verbose: true,
    };
    let engine = SetupEngine::new(config);
    let plan = engine.plan(SetupMode::Create, &request, false)?;

    // Print plan summary
    engine.print_plan(&plan);

    println!("\nPack added to bundle plan. Run setup to configure.");
    Ok(())
}

/// Run the `bundle setup` command.
pub fn setup(args: BundleSetupArgs) -> Result<()> {
    let bundle_dir = resolve_bundle_dir(args.bundle)?;

    // Validate bundle exists
    greentic_setup::bundle::validate_bundle_exists(&bundle_dir)
        .context("invalid bundle directory")?;

    println!("Setting up provider...");
    println!("  Provider: {}", args.provider_id);
    println!("  Bundle: {}", bundle_dir.display());
    println!("  Tenant: {}", args.tenant);
    println!("  Team: {}", args.team.as_deref().unwrap_or("default"));
    println!("  Env: {}", args.env);

    // Load answers if provided
    let setup_answers: serde_json::Map<String, serde_json::Value> =
        if let Some(answers_path) = &args.answers {
            let content = std::fs::read_to_string(answers_path)
                .context("failed to read answers file")?;
            let value: serde_json::Value = if answers_path.extension().map_or(false, |e| e == "yaml" || e == "yml") {
                serde_yaml_bw::from_str(&content)?
            } else {
                serde_json::from_str(&content)?
            };
            match value {
                serde_json::Value::Object(map) => map,
                _ => bail!("answers file must be a JSON/YAML object"),
            }
        } else if args.non_interactive {
            bail!("--answers required in non-interactive mode");
        } else {
            // Interactive mode - would use QA wizard
            println!("\nInteractive setup not yet implemented in CLI.");
            println!("Use --answers <file> to provide setup answers.");
            bail!("interactive setup requires --answers file");
        };

    // Build setup request
    let request = SetupRequest {
        bundle: bundle_dir.clone(),
        providers: vec![args.provider_id.clone()],
        tenants: vec![TenantSelection {
            tenant: args.tenant.clone(),
            team: args.team.clone(),
            allow_paths: Vec::new(),
        }],
        setup_answers,
        ..Default::default()
    };

    // Create engine and build plan
    let config = SetupConfig {
        tenant: args.tenant,
        team: args.team,
        env: args.env,
        offline: false,
        verbose: true,
    };
    let engine = SetupEngine::new(config);
    let plan = engine.plan(SetupMode::Create, &request, false)?;

    // Print plan summary
    engine.print_plan(&plan);

    println!("\nSetup plan created. Provider: {}", args.provider_id);
    Ok(())
}

/// Run the `bundle update` command.
pub fn update(args: BundleSetupArgs) -> Result<()> {
    let bundle_dir = resolve_bundle_dir(args.bundle.clone())?;

    // Validate bundle exists
    greentic_setup::bundle::validate_bundle_exists(&bundle_dir)
        .context("invalid bundle directory")?;

    println!("Updating provider configuration...");

    // Load answers if provided
    let setup_answers: serde_json::Map<String, serde_json::Value> =
        if let Some(answers_path) = &args.answers {
            let content = std::fs::read_to_string(answers_path)
                .context("failed to read answers file")?;
            let value: serde_json::Value = if answers_path.extension().map_or(false, |e| e == "yaml" || e == "yml") {
                serde_yaml_bw::from_str(&content)?
            } else {
                serde_json::from_str(&content)?
            };
            match value {
                serde_json::Value::Object(map) => map,
                _ => bail!("answers file must be a JSON/YAML object"),
            }
        } else if args.non_interactive {
            bail!("--answers required in non-interactive mode");
        } else {
            bail!("interactive update requires --answers file");
        };

    // Build update request
    let request = SetupRequest {
        bundle: bundle_dir.clone(),
        providers: vec![args.provider_id.clone()],
        tenants: vec![TenantSelection {
            tenant: args.tenant.clone(),
            team: args.team.clone(),
            allow_paths: Vec::new(),
        }],
        setup_answers,
        ..Default::default()
    };

    // Create engine and build plan
    let config = SetupConfig {
        tenant: args.tenant,
        team: args.team,
        env: args.env,
        offline: false,
        verbose: true,
    };
    let engine = SetupEngine::new(config);
    let plan = engine.plan(SetupMode::Update, &request, false)?;

    // Print plan summary
    engine.print_plan(&plan);

    println!("\nUpdate plan created. Provider: {}", args.provider_id);
    Ok(())
}

/// Run the `bundle remove` command.
pub fn remove(args: BundleRemoveArgs) -> Result<()> {
    let bundle_dir = resolve_bundle_dir(args.bundle)?;

    // Validate bundle exists
    greentic_setup::bundle::validate_bundle_exists(&bundle_dir)
        .context("invalid bundle directory")?;

    println!("Removing provider...");
    println!("  Provider: {}", args.provider_id);
    println!("  Bundle: {}", bundle_dir.display());

    if !args.force {
        println!("\nThis will remove the provider configuration.");
        println!("Use --force to confirm.");
        bail!("removal cancelled - use --force to confirm");
    }

    // Build remove request
    let request = SetupRequest {
        bundle: bundle_dir.clone(),
        providers_remove: vec![args.provider_id.clone()],
        tenants: vec![TenantSelection {
            tenant: args.tenant.clone(),
            team: args.team.clone(),
            allow_paths: Vec::new(),
        }],
        ..Default::default()
    };

    // Create engine and build plan
    let config = SetupConfig {
        tenant: args.tenant,
        team: args.team,
        env: "dev".to_string(),
        offline: false,
        verbose: true,
    };
    let engine = SetupEngine::new(config);
    let plan = engine.plan(SetupMode::Remove, &request, false)?;

    // Print plan summary
    engine.print_plan(&plan);

    println!("\nProvider removed: {}", args.provider_id);
    Ok(())
}

/// Run the `bundle status` command.
pub fn status(args: BundleStatusArgs) -> Result<()> {
    let bundle_dir = resolve_bundle_dir(args.bundle)?;

    if !bundle_dir.exists() {
        if args.format == "json" {
            println!(r#"{{"exists": false, "path": "{}"}}"#, bundle_dir.display());
        } else {
            println!("Bundle not found: {}", bundle_dir.display());
        }
        return Ok(());
    }

    // Check if valid bundle
    let is_valid = bundle_dir.join("greentic.demo.yaml").exists();

    // Count providers in providers/ directory
    let providers_dir = bundle_dir.join("providers");
    let mut pack_count = 0;
    let mut packs = Vec::new();

    if providers_dir.exists() {
        for domain in &["messaging", "events", "oauth", "secrets", "mcp", "state", "other"] {
            let domain_dir = providers_dir.join(domain);
            if domain_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&domain_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map_or(false, |e| e == "gtpack") {
                            pack_count += 1;
                            if let Some(name) = path.file_stem().and_then(|n| n.to_str()) {
                                packs.push(format!("{}/{}", domain, name));
                            }
                        }
                    }
                }
            }
        }
    }

    // Count tenants
    let tenants_dir = bundle_dir.join("tenants");
    let mut tenant_count = 0;
    let mut tenants = Vec::new();

    if tenants_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&tenants_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    tenant_count += 1;
                    if let Some(name) = entry.file_name().to_str() {
                        tenants.push(name.to_string());
                    }
                }
            }
        }
    }

    if args.format == "json" {
        let status = serde_json::json!({
            "exists": true,
            "valid": is_valid,
            "path": bundle_dir.display().to_string(),
            "pack_count": pack_count,
            "packs": packs,
            "tenant_count": tenant_count,
            "tenants": tenants,
        });
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!("Bundle: {}", bundle_dir.display());
        println!("Valid: {}", if is_valid { "yes" } else { "no (missing greentic.demo.yaml)" });
        println!("Packs: {} installed", pack_count);
        for pack in &packs {
            println!("  - {}", pack);
        }
        println!("Tenants: {}", tenant_count);
        for tenant in &tenants {
            println!("  - {}", tenant);
        }
    }

    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn resolve_bundle_dir(bundle: Option<PathBuf>) -> Result<PathBuf> {
    match bundle {
        Some(path) => Ok(path),
        None => std::env::current_dir().context("failed to get current directory"),
    }
}
