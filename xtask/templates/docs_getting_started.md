# {{component_pascal}} Component – Getting Started

Welcome to the `{{component_name}}` component scaffold! This guide walks you through building, testing, and describing the component.

## Build

```bash
cargo build
```

The scaffold uses stable Rust and keeps dependencies minimal.

## Run tests

```bash
cargo test
```

The template ships with a schema validation test to ensure the example flow stays in sync with the JSON Schema.

## Describe API

The entry point lives at `src/describe.rs` and currently loads the component schema from:

```
schemas/v1/{{component_kebab}}.node.schema.json
```

Eventually the component binary or pack should expose a describe endpoint (typically `component describe`). Use the scaffolded JSON Schema as the source of truth for input validation.

## Schema URL

Publish the schema at:

```
https://greenticai.github.io/component-{{component_kebab}}/schemas/v1/{{component_kebab}}.node.schema.json
```

The placeholder value is already embedded in the describe payload.

