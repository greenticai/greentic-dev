PR-DEV-TENANT-INSTALL-02
Title

Extend greentic-dev install --tenant <tenant> to fetch tenant OCI manifests and install release binaries and local docs

Summary

Add a minimum tenant-aware install flow to greentic-dev so that:

greentic-dev install --tenant acme --token env:GITHUB_TOKEN

can:

pull the tenant OCI JSON manifest from customers-tools

resolve the allowed tool/doc IDs

download commercial release binaries

install binaries into a local PATH directory

download and install docs locally

optionally install docs into a specified directory

This PR should preserve the existing OSS flow:

greentic-dev install tools

The new tenant-aware behavior should live in the bare install path, not in install tools.

Minimum CLI design

Support:

greentic-dev install
greentic-dev install --tenant <tenant> --token <token-or-env:VAR>
greentic-dev install --tenant <tenant> --token <token-or-env:VAR> --docs-dir <dir>
greentic-dev install --tenant <tenant> --token <token-or-env:VAR> --bin-dir <dir>
Behavior
No tenant

install OSS catalog only, if implemented in bare install

or no-op with guidance if bare install is introduced gradually

With tenant

fetch tenant OCI JSON manifest

resolve tools/docs

install allowed commercial tools/docs

Binary installation model

Commercial tools are release binaries, so the installer should:

detect platform:

OS

architecture

choose the matching release target from the tool manifest

download the archive or binary

verify checksum

extract if needed

copy executable into install directory

Default bin directory

Use a user-local directory such as:

~/.greentic/bin

This directory should be added to PATH by the user if not already present.

Optional bin directory override

Support:

--bin-dir <dir>

If set, install there instead.

Docs installation model

Docs should be installed locally:

Default docs directory
~/.greentic/install/docs
Optional docs directory override

Support:

--docs-dir <dir>

If set, install docs there instead.

Docs format

MVP can support:

markdown

plain files fetched by URL

Install docs by downloading the source and writing to the target path.

Required work
1. Add bare install path

Update CLI so bare install becomes the new manifest-driven install path.

Preserve:

greentic-dev install tools

as the legacy OSS path.

2. Add tenant OCI fetcher

Implement tenant manifest retrieval from:

oci://ghcr.io/greentic-biz/customers-tools/<tenant>:latest

using the supplied GitHub token.

This should return the tenant JSON manifest.

3. Add manifest resolution

After pulling the tenant manifest:

resolve referenced tool manifests

resolve referenced doc manifests

How you do this can be one of two minimum approaches:

Simpler MVP

The tenant OCI JSON already contains fully expanded tool/doc install entries.

Slightly richer MVP

The tenant OCI JSON contains IDs and greentic-dev fetches the referenced manifests from GitHub raw or repo contents.

Recommendation: use the simpler MVP and make the tenant OCI manifest fully expanded.
That reduces round-trips and keeps the first installer simpler.

So update PR 1 accordingly: publish expanded tenant OCI manifests.

4. Add binary installer

Implement a release-binary installer that supports:

direct binary downloads

.tar.gz

.zip if already easy in repo dependencies

At minimum:

Linux

macOS

Windows can be added if already straightforward.

5. Add checksum verification

Require sha256 in release targets and verify downloads before install.

Fail if checksum mismatches.

6. Add docs installer

Implement a doc installer that:

downloads doc content

installs into default or specified docs dir

preserves a predictable relative path

7. Add local install state

Track installed tenant manifest and installed artifacts under:

~/.greentic/install/

Suggested minimum:

~/.greentic/install/
├─ manifests/
│  └─ tenant-acme.json
├─ docs/
├─ downloads/
└─ state.json

This does not need to be complex in MVP.

Recommended manifest shape for the installed OCI JSON

For the minimum consumer flow, the tenant OCI artifact should already contain expanded entries, for example:

{
  "schema_version": "1",
  "tenant": "acme",
  "tools": [
    {
      "id": "greentic-x-cli",
      "name": "Greentic X CLI",
      "install": {
        "type": "release-binary",
        "binary_name": "greentic-x",
        "targets": [
          {
            "os": "linux",
            "arch": "x86_64",
            "url": "https://github.com/greentic-biz/greentic-x/releases/download/v1.2.3/greentic-x-linux-x86_64.tar.gz",
            "sha256": "..."
          }
        ]
      }
    }
  ],
  "docs": [
    {
      "id": "acme-onboarding",
      "title": "Acme onboarding",
      "install": {
        "type": "doc",
        "url": "https://raw.githubusercontent.com/greentic-biz/customers-tools/main/docs/acme/onboarding.md",
        "relative_path": "acme/onboarding/README.md"
      }
    }
  ]
}

That is the easiest minimum end-to-end system.

Tests
Required tests

tenant OCI manifest parsing

target selection by OS/arch

checksum verification success/failure

archive extraction test

doc install test

install path override test

tenant install happy path

conflict/error handling when manifest is malformed

Mock HTTP/OCI is fine for MVP tests.

Acceptance criteria

 bare greentic-dev install --tenant <tenant> --token ... works

 tenant OCI JSON is pulled and parsed

 release-binary tools are downloaded and installed into default or specified bin dir

 checksums are verified

 docs are installed into default or specified docs dir

 install tools still works unchanged

 tests cover binary + docs flows

Recommended small design decisions
1. Expanded tenant OCI manifest

For MVP, publish a fully expanded tenant manifest in OCI, not just IDs.

That makes greentic-dev much simpler.

2. Default local directories

Use:

binaries: ~/.greentic/bin

docs: ~/.greentic/install/docs

3. Optional directory overrides

Support:

--bin-dir

--docs-dir

4. Keep OSS and commercial paths separate

install tools = current OSS path

bare install --tenant ... = new commercial path

That keeps behavior easy to reason about.

Final short architecture
greentic-biz/customers-tools

Publishes tenant-specific expanded OCI JSON manifests.

greentic-dev

Consumes those manifests and installs:

release binaries into PATH-friendly local bin dir

docs into local docs dir

GitHub token

Used to access the private OCI manifest and, if needed, private release assets.

If you want, I can now rewrite both PRs in an even tighter Codex prompt format with explicit task lists and acceptance checkboxes only.