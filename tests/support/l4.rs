use std::io::Write;
use std::str::FromStr;

use anyhow::Result;
use greentic_types::flow::{
    ComponentRef, Flow, FlowHasher, FlowKind, FlowMetadata, InputMapping, Node, OutputMapping,
    Routing,
};
use greentic_types::{ComponentId, FlowId, NodeId, PackFlowEntry, PackId, PackKind, PackManifest};
use semver::Version;
use serde_json::json;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

pub fn build_l4_pack() -> Result<Vec<u8>> {
    let flow = build_flow()?;
    let manifest = PackManifest {
        schema_version: "1".to_string(),
        pack_id: PackId::from_str("dev.local.l4")?,
        name: None,
        version: Version::parse("0.1.0")?,
        kind: PackKind::Application,
        publisher: "test".into(),
        components: Vec::new(),
        flows: vec![PackFlowEntry {
            id: flow.id.clone(),
            kind: flow.kind,
            flow,
            tags: Vec::new(),
            entrypoints: Vec::new(),
        }],
        dependencies: Vec::new(),
        capabilities: Vec::new(),
        secret_requirements: Vec::new(),
        signatures: Default::default(),
        bootstrap: None,
        extensions: None,
        agents: Default::default(),
    };
    let manifest_bytes = greentic_types::encode_pack_manifest(&manifest)?;

    let mut buf = Vec::new();
    {
        let mut writer = ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts = SimpleFileOptions::default();
        writer.start_file("manifest.cbor", opts)?;
        writer.write_all(&manifest_bytes)?;
        writer.start_file("components/tool-secret.wasm", opts)?;
        writer.write_all(&[0u8; 8])?;
        writer.start_file("components/tool-external.wasm", opts)?;
        writer.write_all(&[0u8; 8])?;
        writer.finish()?;
    }
    Ok(buf)
}

fn build_flow() -> Result<Flow> {
    let start_id: NodeId = "start".parse()?;
    let secret_id: NodeId = "secret".parse()?;
    let external_id: NodeId = "external".parse()?;
    let finish_id: NodeId = "finish".parse()?;
    let err_id: NodeId = "err".parse()?;

    let start = Node {
        id: start_id.clone(),
        component: ComponentRef {
            id: ComponentId::from_str("component.start")?,
            pack_alias: None,
            operation: None,
        },
        input: InputMapping { mapping: json!({}) },
        output: OutputMapping { mapping: json!({}) },
        err_map: None,
        routing: Routing::Next {
            node_id: secret_id.clone(),
        },
        telemetry: Default::default(),
        conversational: false,
    };
    let secret = Node {
        id: secret_id.clone(),
        component: ComponentRef {
            id: ComponentId::from_str("component.tool.secret")?,
            pack_alias: None,
            operation: None,
        },
        input: InputMapping { mapping: json!({}) },
        output: OutputMapping { mapping: json!({}) },
        err_map: None,
        routing: Routing::Branch {
            on_status: [("error".to_string(), err_id.clone())]
                .into_iter()
                .collect(),
            default: Some(external_id.clone()),
        },
        telemetry: Default::default(),
        conversational: false,
    };
    let external = Node {
        id: external_id.clone(),
        component: ComponentRef {
            id: ComponentId::from_str("component.tool.external")?,
            pack_alias: None,
            operation: None,
        },
        input: InputMapping {
            mapping: json!({ "url": "https://example.com/resource" }),
        },
        output: OutputMapping { mapping: json!({}) },
        err_map: None,
        routing: Routing::Branch {
            on_status: [("error".to_string(), err_id.clone())]
                .into_iter()
                .collect(),
            default: Some(finish_id.clone()),
        },
        telemetry: Default::default(),
        conversational: false,
    };
    let finish = Node {
        id: finish_id.clone(),
        component: ComponentRef {
            id: ComponentId::from_str("component.template")?,
            pack_alias: None,
            operation: None,
        },
        input: InputMapping { mapping: json!({}) },
        output: OutputMapping { mapping: json!({}) },
        err_map: None,
        routing: Routing::End,
        telemetry: Default::default(),
        conversational: false,
    };
    let err = Node {
        id: err_id.clone(),
        component: ComponentRef {
            id: ComponentId::from_str("component.error.map")?,
            pack_alias: None,
            operation: None,
        },
        input: InputMapping { mapping: json!({}) },
        output: OutputMapping { mapping: json!({}) },
        err_map: None,
        routing: Routing::End,
        telemetry: Default::default(),
        conversational: false,
    };

    let mut nodes: indexmap::IndexMap<NodeId, Node, FlowHasher> = indexmap::IndexMap::default();
    nodes.insert(start_id, start);
    nodes.insert(secret_id, secret);
    nodes.insert(external_id, external);
    nodes.insert(finish_id, finish);
    nodes.insert(err_id, err);

    Ok(Flow {
        schema_version: "1".into(),
        id: FlowId::from_str("default")?,
        kind: FlowKind::ComponentConfig,
        entrypoints: Default::default(),
        nodes,
        metadata: FlowMetadata::default(),
    })
}
