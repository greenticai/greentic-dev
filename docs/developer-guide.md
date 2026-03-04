# Developer Guide: End-to-End Hello World Component

This guide walks from a clean workstation all the way to a “hello world” Greentic component that you can validate locally. Follow the steps in order; each section calls out the exact commands you need to run.

---

## 1. Prerequisites

1. **Install Rust (via rustup).**
   ```bash
   curl https://sh.rustup.rs -sSf | sh
   source "$HOME/.cargo/env"
   rustup update stable
   ```

2. **Add the WASI target required by `cargo component`.**
   ```bash
   rustup target add wasm32-wasip2
   ```

3. **Install component tooling.**
   ```bash
   cargo install cargo-component --locked
   ```
   > `cargo-component` performs the WIT-driven build; install it once and keep it updated with `cargo install cargo-component --locked --force`.

4. **(Optional) Install supporting CLI tools.**
   - `cargo install wasm-tools --locked` if you want to inspect component metadata with `wasm-tools component wit`.
   - `cargo install just` if you prefer using the repo’s shorthand tasks.

5. **(Optional) Install `greentic-dev` globally.**
   ```bash
   cargo binstall greentic-dev
   ```
   For the latest commit or local forks:
   ```bash
   cargo install --git https://github.com/greenticai/greentic-dev greentic-dev
   # or from the current checkout
   cargo install --path .
   ```
   Installing the CLI lets you run `greentic-dev component …` without prefixing commands with `cargo run -p`.

> For most workflows (including this guide) you only need the installed CLI. Clone the `greentic-dev` repository only if you intend to contribute code or work on the tooling itself.

---

## 2. Build a component, then a pack (end-to-end)

The happy path is entirely CLI-driven:

1) **Scaffold the pack workspace.**
```bash
greentic-dev pack new --dir ./hello-pack dev.local.hello-pack
cd hello-pack
```
2) **Scaffold the component inside the pack.**
```bash
greentic-dev component new --name hello-world --path ./components/hello-world --non-interactive --no-git --no-check
```
3) **Build and doctor the component.** (Doctor validates the component is correctly formatted and needs either a colocated manifest or an explicit `--manifest`.)
```bash
greentic-dev component build --manifest components/hello-world/component.manifest.json
greentic-dev component doctor components/hello-world/target/wasm32-wasip2/release/hello_world.wasm \
  --manifest components/hello-world/component.manifest.json
```
4) **Add the main flow and the component to the flow.** This wires your built component into the default flow using greentic-flow via greentic-dev.
```bash
greentic-dev flow add-step \
  --flow flows/main.ygtc \
  --local-wasm  components/hello-world/target/wasm32-wasip2/release/hello_world.wasm \
  --node-id hello-world \
  --operation handle_message \
  --payload '{"input": "Hello from hello-world!"}' \
  --routing-out
```
> Tip: if your manifest defines the operation, you can omit `--operation`; `--payload`/`--routing` can also be omitted for the default shape.

5) **Sync pack.yaml components from the components/ directory.** This uses the underlying `greentic-pack components` to add your built component entry into `pack.yaml`.
```bash
greentic-dev pack components --in .
```
6) **Validate the flow.**
```bash
greentic-dev flow doctor flows/main.ygtc
```
7) **Build and check the pack.**
```bash
greentic-dev pack build --in . --gtpack-out dist/hello.gtpack
greentic-dev pack doctor dist/hello.gtpack
```

8) **Run the pack.**
```bash
greentic-dev pack run --pack dist/hello.gtpack --artifacts dist/artifacts
```

That sequence yields a runnable pack that pulls a config-flow-defined node from your component, bundles it, and executes it locally without touching the network.

> If doctor/pack build fails, double-check: (a) the WASM is a component (built with `cargo component`), (b) `component.manifest.json` includes `dev_flows.default`, and (c) the pack’s `pack.yaml` references your component artifact.

---

## 3. Hello2-pack with a remote templates component (OCI)

If you want to wire a remote component into your pack and flow, you can reference it via OCI and keep it in `pack.yaml` under the components extension.

1) **Scaffold a second pack workspace.**
```bash
greentic-dev pack new --dir ./hello2-pack dev.local.hello2-pack
cd hello2-pack
```
2) **Generate answers for the remote templates component and add the resulting node.**
```bash
mkdir -p flows/answers
greentic-dev flow answers \
  --component oci://ghcr.io/greenticai/components/templates:latest \
  --operation text \
  --mode config \
  --name templates \
  --out-dir flows/answers
greentic-dev flow add-step \
  --flow flows/main.ygtc \
  --node-id templates \
  --operation text \
  --mode config \
  --component oci://ghcr.io/greenticai/components/templates:latest \
  --answers-file flows/answers/templates.example.json \
  --routing-out
```
> The templates `text` operation expects a `config` object; confirm the generated `templates.example.json` includes a top-level `config` field containing the `text` you want to render before running `add-step`.
`flow answers` writes `flows/answers/templates.schema.json` and `flows/answers/templates.example.json`; edit the example JSON to ensure you provide a full `ChannelMessageEnvelope` for both `config.msg` and `msg` (the schema mirrors `greentic_types::ChannelMessageEnvelope`, e.g., `id`, `tenant.{env,tenant,tenant_id}`, `channel`, `session_id`, `text`, `metadata`, etc.) before re-running `add-step` with `--answers-file`. Use `--pin` on `flow add-step` to resolve the OCI tag to a digest if you want reproducible builds.


3) **Optionally: Validate the flow and pack.** (`:latest` requires `--allow-oci-tags`.)
```bash
greentic-dev flow doctor flows/main.ygtc
```
4) **Build and run the pack.** (Requires network access to pull the remote component.)
```bash
greentic-dev pack build --in . --gtpack-out dist/hello2.gtpack --allow-oci-tags
greentic-dev pack doctor --pack dist/hello2.gtpack
greentic-dev pack run --pack dist/hello2.gtpack --artifacts dist/artifacts
```
> Offline test rigs sometimes skip this last command because the templates component expects runtime metadata we cannot provide artificially; if a real registry is accessible the run should succeed once the manifest's `config` payload matches `templates.schema.json`.

---

## Quick reference

```
# one-time setup
rustup target add wasm32-wasip2
cargo install cargo-component --locked

# scaffold + build + doctor
greentic-dev pack new -- --dir ./hello-pack dev.local.hello-pack
cd hello-pack
greentic-dev component new --name hello-world --path ./components/hello-world --non-interactive --no-git --no-check
GREENTIC_DEV_OFFLINE=1 CARGO_NET_OFFLINE=true greentic-dev component build --manifest components/hello-world/component.manifest.json
greentic-dev component doctor components/hello-world/target/wasm32-wasip2/release/component_hello_world.wasm \
  --manifest components/hello-world/component.manifest.json

# pack + run
greentic-dev pack components -- --in .
greentic-dev flow doctor flows/main.ygtc --json
greentic-dev pack doctor --pack pack.yaml
greentic-dev pack build -- --in . --gtpack-out dist/hello.gtpack
greentic-dev pack run --pack dist/hello.gtpack --offline

# optional: register and inspect provider extensions
# (this lives outside the main flow wiring steps)
# greentic-dev pack new-provider --pack manifest.cbor --id dev.local.hello.provider --runtime components/hello-world::greentic_provider@greentic:provider/runtime --manifest providers/dev.local.hello.provider/provider.yaml --kind demo
# greentic-pack providers list dist/hello.gtpack
# greentic-pack providers info dist/hello.gtpack --id dev.local.hello.provider
# greentic-pack providers validate dist/hello.gtpack
```

