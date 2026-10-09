#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolchainPackageSpec {
    pub crate_name: &'static str,
    pub bins: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OciPackageSpec {
    pub package: &'static str,
}

pub const GREENTIC_TOOLCHAIN_PACKAGES: &[ToolchainPackageSpec] = &[
    ToolchainPackageSpec {
        crate_name: "greentic-dev",
        bins: &["greentic-dev"],
    },
    ToolchainPackageSpec {
        crate_name: "greentic-operator",
        bins: &["greentic-operator"],
    },
    ToolchainPackageSpec {
        crate_name: "greentic-bundle",
        bins: &["greentic-bundle"],
    },
    ToolchainPackageSpec {
        crate_name: "greentic-setup",
        bins: &["greentic-setup"],
    },
    ToolchainPackageSpec {
        crate_name: "greentic-start",
        bins: &["greentic-start"],
    },
    ToolchainPackageSpec {
        crate_name: "greentic-deployer",
        bins: &["greentic-deployer"],
    },
    ToolchainPackageSpec {
        crate_name: "greentic-component",
        bins: &["greentic-component"],
    },
    ToolchainPackageSpec {
        crate_name: "greentic-flow",
        bins: &["greentic-flow"],
    },
    ToolchainPackageSpec {
        crate_name: "greentic-pack",
        bins: &["greentic-pack"],
    },
    ToolchainPackageSpec {
        crate_name: "greentic-runner",
        bins: &["greentic-runner"],
    },
    ToolchainPackageSpec {
        crate_name: "greentic-gui",
        bins: &["greentic-gui"],
    },
    ToolchainPackageSpec {
        crate_name: "greentic-secrets",
        bins: &["greentic-secrets"],
    },
    ToolchainPackageSpec {
        crate_name: "greentic-mcp",
        bins: &["greentic-mcp"],
    },
];

/// External tools distributed as their own single, unsuffixed binary
/// (not part of the Greentic channel-suffixed toolchain). Resolved and
/// installed by plain name — never with a `-dev`/`-rnd` suffix.
pub const GREENTIC_EXTERNAL_TOOL_PACKAGES: &[ToolchainPackageSpec] = &[];

pub const GREENTIC_EXTENSION_PACK_PACKAGES: &[OciPackageSpec] = &[
    OciPackageSpec {
        package: "packs/demos/cards-demo",
    },
    OciPackageSpec {
        package: "packs/demos/cloud-deploy-demo-app",
    },
    OciPackageSpec {
        package: "packs/demos/deep-research-demo",
    },
    OciPackageSpec {
        package: "packs/demos/helpdesk-itsm",
    },
    OciPackageSpec {
        package: "packs/demos/hr-onboarding",
    },
    OciPackageSpec {
        package: "packs/demos/incident-demo",
    },
    OciPackageSpec {
        package: "packs/demos/quickstart",
    },
    OciPackageSpec {
        package: "packs/demos/quickstart-event",
    },
    OciPackageSpec {
        package: "packs/demos/redbutton-demo",
    },
    OciPackageSpec {
        package: "packs/demos/sales-crm",
    },
    OciPackageSpec {
        package: "packs/demos/supply-chain",
    },
    OciPackageSpec {
        package: "packs/demos/telco-x",
    },
    OciPackageSpec {
        package: "packs/demos/weather-mcp-demo",
    },
    OciPackageSpec {
        package: "packs/demos/weatherapi-pack",
    },
    OciPackageSpec {
        package: "packs/deployer/greentic.deploy.gcp",
    },
    OciPackageSpec {
        package: "packs/deployer/greentic.deploy.azure",
    },
    OciPackageSpec {
        package: "packs/deployer/greentic.deploy.aws",
    },
    OciPackageSpec {
        package: "packs/deployer/greentic.deploy.helm",
    },
    OciPackageSpec {
        package: "packs/deployer/greentic.deploy.juju-k8s",
    },
    OciPackageSpec {
        package: "packs/deployer/greentic.deploy.juju-machine",
    },
    OciPackageSpec {
        package: "packs/deployer/greentic.deploy.k8s-raw",
    },
    OciPackageSpec {
        package: "packs/deployer/greentic.deploy.serverless",
    },
    OciPackageSpec {
        package: "packs/deployer/greentic.deploy.snap",
    },
    OciPackageSpec {
        package: "packs/deployer/greentic.deploy.terraform",
    },
    OciPackageSpec {
        package: "packs/dw/context/compressor-pack",
    },
    OciPackageSpec {
        package: "packs/dw/context/retrieval-pack",
    },
    OciPackageSpec {
        package: "packs/dw/context/static-pack",
    },
    OciPackageSpec {
        package: "packs/dw/control/basic-policy-pack",
    },
    OciPackageSpec {
        package: "packs/dw/control/delegation-guard-pack",
    },
    OciPackageSpec {
        package: "packs/dw/delegation/capability-match-pack",
    },
    OciPackageSpec {
        package: "packs/dw/delegation/static-router-pack",
    },
    OciPackageSpec {
        package: "packs/dw/engine/default-pack",
    },
    OciPackageSpec {
        package: "packs/dw/engine/router-lite-pack",
    },
    OciPackageSpec {
        package: "packs/dw/memory/short-term-in-memory-pack",
    },
    OciPackageSpec {
        package: "packs/dw/memory/short-term-redis-pack",
    },
    OciPackageSpec {
        package: "packs/dw/observer/basic-audit-pack",
    },
    OciPackageSpec {
        package: "packs/dw/observer/basic-metrics-pack",
    },
    OciPackageSpec {
        package: "packs/dw/planning/llm-outline-pack",
    },
    OciPackageSpec {
        package: "packs/dw/planning/static-pack",
    },
    OciPackageSpec {
        package: "packs/dw/reflection/llm-critic-pack",
    },
    OciPackageSpec {
        package: "packs/dw/reflection/rules-pack",
    },
    OciPackageSpec {
        package: "packs/dw/reflection/schema-check-pack",
    },
    OciPackageSpec {
        package: "packs/dw/state/task-store-in-memory-pack",
    },
    OciPackageSpec {
        package: "packs/dw/state/task-store-redis-pack",
    },
    OciPackageSpec {
        package: "packs/dw/tool/component-adapter-pack",
    },
    OciPackageSpec {
        package: "packs/dw/tool/mcp-adapter-pack",
    },
    OciPackageSpec {
        package: "packs/dw/workspace/fs-pack",
    },
    OciPackageSpec {
        package: "packs/dw/workspace/in-memory-pack",
    },
    OciPackageSpec {
        package: "packs/events/events-dummy",
    },
    OciPackageSpec {
        package: "packs/events/events-email",
    },
    OciPackageSpec {
        package: "packs/events/events-email-sendgrid",
    },
    OciPackageSpec {
        package: "packs/events/events-sms",
    },
    OciPackageSpec {
        package: "packs/events/events-sms-twilio",
    },
    OciPackageSpec {
        package: "packs/events/events-timer",
    },
    OciPackageSpec {
        package: "packs/events/events-webhook",
    },
    OciPackageSpec {
        package: "packs/messaging/messaging-dummy",
    },
    OciPackageSpec {
        package: "packs/messaging/messaging-email",
    },
    OciPackageSpec {
        package: "packs/messaging/messaging-http",
    },
    OciPackageSpec {
        package: "packs/messaging/messaging-slack",
    },
    OciPackageSpec {
        package: "packs/messaging/messaging-teams",
    },
    OciPackageSpec {
        package: "packs/messaging/messaging-telegram",
    },
    OciPackageSpec {
        package: "packs/messaging/messaging-webchat",
    },
    OciPackageSpec {
        package: "packs/messaging/messaging-webchat-gui",
    },
    OciPackageSpec {
        package: "packs/messaging/messaging-webex",
    },
    OciPackageSpec {
        package: "packs/messaging/messaging-websocket",
    },
    OciPackageSpec {
        package: "packs/messaging/messaging-whatsapp",
    },
    OciPackageSpec {
        package: "packs/oauth/oauth-github",
    },
    OciPackageSpec {
        package: "packs/oauth/oauth-google",
    },
    OciPackageSpec {
        package: "packs/oauth/oauth-microsoft-graph",
    },
    OciPackageSpec {
        package: "packs/oauth/oauth-oidc-generic",
    },
    OciPackageSpec {
        package: "packs/oauth/oauth-slack",
    },
    OciPackageSpec {
        package: "packs/secret/greentic.secrets.aws-sm.gtpack",
    },
    OciPackageSpec {
        package: "packs/secret/greentic.secrets.azure-kv.gtpack",
    },
    OciPackageSpec {
        package: "packs/secret/greentic.secrets.gcp-sm.gtpack",
    },
    OciPackageSpec {
        package: "packs/secret/greentic.secrets.k8s.gtpack",
    },
    OciPackageSpec {
        package: "packs/secret/greentic.secrets.providers.gtpack",
    },
    OciPackageSpec {
        package: "packs/secret/greentic.secrets.vault-kv.gtpack",
    },
    OciPackageSpec {
        package: "packs/state-memory",
    },
    OciPackageSpec {
        package: "packs/state-redis",
    },
    OciPackageSpec {
        package: "packs/state/state-memory",
    },
    OciPackageSpec {
        package: "packs/state/state-redis",
    },
];
pub const GREENTIC_COMPONENT_PACKAGES: &[OciPackageSpec] = &[
    OciPackageSpec {
        package: "component/component-events2msg",
    },
    OciPackageSpec {
        package: "component/component-http",
    },
    OciPackageSpec {
        package: "component/component-llm-openai",
    },
    OciPackageSpec {
        package: "component/component-msg2events",
    },
    OciPackageSpec {
        package: "component/component-pack2flow",
    },
    OciPackageSpec {
        package: "component/component-sorx-business",
    },
    OciPackageSpec {
        package: "components/component-adaptive-card",
    },
    OciPackageSpec {
        package: "components/component-qa",
    },
    OciPackageSpec {
        package: "components/secrets-provider-inmemory",
    },
    OciPackageSpec {
        package: "components/templates",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn external_packages_exclude_mcp_generator() {
        let found = GREENTIC_EXTERNAL_TOOL_PACKAGES
            .iter()
            .find(|pkg| pkg.crate_name == "greentic-mcp-generator");
        assert!(
            found.is_none(),
            "generator must not be installed by binstall"
        );
    }

    #[test]
    fn catalogue_contains_expected_public_toolchain() {
        let expected = [
            ("greentic-dev", "greentic-dev"),
            ("greentic-operator", "greentic-operator"),
            ("greentic-bundle", "greentic-bundle"),
            ("greentic-setup", "greentic-setup"),
            ("greentic-start", "greentic-start"),
            ("greentic-deployer", "greentic-deployer"),
            ("greentic-component", "greentic-component"),
            ("greentic-flow", "greentic-flow"),
            ("greentic-pack", "greentic-pack"),
            ("greentic-runner", "greentic-runner"),
            ("greentic-gui", "greentic-gui"),
            ("greentic-secrets", "greentic-secrets"),
            ("greentic-mcp", "greentic-mcp"),
        ];

        let actual = catalogue_pairs();
        for pair in expected {
            assert!(actual.contains(&pair), "missing {pair:?}");
        }
    }

    #[test]
    fn catalogue_has_no_duplicate_crate_bin_pairs() {
        let mut seen = BTreeSet::new();
        for package in GREENTIC_TOOLCHAIN_PACKAGES {
            for bin in package.bins {
                assert!(
                    seen.insert((package.crate_name, *bin)),
                    "duplicate crate/bin pair: {}/{}",
                    package.crate_name,
                    bin
                );
            }
        }
    }

    #[test]
    fn extension_pack_catalogue_tracks_github_packages() {
        assert_eq!(GREENTIC_EXTENSION_PACK_PACKAGES.len(), 81);
        assert_catalogue_has_no_duplicate_packages(GREENTIC_EXTENSION_PACK_PACKAGES);
        assert!(GREENTIC_EXTENSION_PACK_PACKAGES.iter().all(|package| {
            package.package.starts_with("packs/") || package.package.starts_with("greentic-bundle/")
        }));
        assert!(
            GREENTIC_EXTENSION_PACK_PACKAGES
                .iter()
                .any(|package| package.package == "packs/deployer/greentic.deploy.aws")
        );
        assert!(
            GREENTIC_EXTENSION_PACK_PACKAGES
                .iter()
                .any(|package| package.package == "packs/deployer/greentic.deploy.helm")
        );
        assert!(GREENTIC_EXTENSION_PACK_PACKAGES.iter().all(|package| {
            !package
                .package
                .starts_with("packs/deployer/greentic.fixture.")
        }));
        assert!(
            GREENTIC_EXTENSION_PACK_PACKAGES
                .iter()
                .any(|package| package.package == "packs/messaging/messaging-webchat-gui")
        );
        assert!(
            GREENTIC_EXTENSION_PACK_PACKAGES
                .iter()
                .any(|package| package.package == "packs/messaging/messaging-http")
        );
        assert!(
            GREENTIC_EXTENSION_PACK_PACKAGES
                .iter()
                .any(|package| package.package == "packs/messaging/messaging-websocket")
        );
        assert!(
            GREENTIC_EXTENSION_PACK_PACKAGES
                .iter()
                .any(|package| package.package == "packs/oauth/oauth-microsoft-graph")
        );
    }

    #[test]
    fn component_catalogue_tracks_github_packages() {
        assert_eq!(GREENTIC_COMPONENT_PACKAGES.len(), 10);
        assert_catalogue_has_no_duplicate_packages(GREENTIC_COMPONENT_PACKAGES);
        assert!(GREENTIC_COMPONENT_PACKAGES.iter().all(|package| {
            package.package.starts_with("component/")
                || package.package.starts_with("components/")
                || package.package.starts_with("validators/")
        }));
        assert!(
            GREENTIC_COMPONENT_PACKAGES
                .iter()
                .any(|package| package.package == "component/component-llm-openai")
        );
        assert!(
            GREENTIC_COMPONENT_PACKAGES
                .iter()
                .any(|package| package.package == "component/component-sorx-business")
        );
        assert!(
            GREENTIC_COMPONENT_PACKAGES
                .iter()
                .any(|package| package.package == "components/component-adaptive-card")
        );
    }

    fn catalogue_pairs() -> BTreeSet<(&'static str, &'static str)> {
        GREENTIC_TOOLCHAIN_PACKAGES
            .iter()
            .flat_map(|package| {
                package
                    .bins
                    .iter()
                    .map(move |bin| (package.crate_name, *bin))
            })
            .collect()
    }

    fn assert_catalogue_has_no_duplicate_packages(catalogue: &[OciPackageSpec]) {
        let mut seen = BTreeSet::new();
        for package in catalogue {
            assert!(
                seen.insert(package.package),
                "duplicate package: {}",
                package.package
            );
        }
    }
}
