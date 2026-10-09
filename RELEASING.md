## Releasing greentic-dev

1. Update `Cargo.toml` with the new version and land the change on `master`.
2. Run `ci/local_check.sh` to make sure fmt/clippy/tests/build all pass locally.
3. Tag the commit with the final version (`git tag vX.Y.Z && git push origin vX.Y.Z`).
4. GitHub Actions will run the `Release` workflow for the tag: it builds binaries for all supported targets, uploads them to the GitHub Release so `cargo binstall` can fetch them, and then runs the publish stage for crates.io.

The publish workflow verifies that the tag version matches the crate version, so avoid pushing mismatched tags.

## Dev channel builds

Every push to `develop` runs `Dev Publish`, which builds binaries through
`greenticai/.github`'s `dev-release-binaries.yml` and attaches them to a
prerelease tagged `v<major>.<minor>.<run-id>`. Since greenticai/.github#262 the
binary is built at that same version, so `greentic-dev-dev --version` reports
`<major>.<minor>.<run-id>`; builds from before it report the develop base
(`1.2.0-dev.0`).

While crates.io publishing is unavailable, the `gtc:dev` channel is republished
from those GitHub releases:

```bash
greentic-dev release snapshot --release <version> --channel dev \
  --source github-releases --tag dev --dry-run
greentic-dev release snapshot --release <version> --channel dev \
  --source github-releases --tag dev
```

`<version>` names the pushed `gtc:<version>` manifest; by convention it is the
greentic-dev dev version the channel ships (for example `1.2.34841799250`).

Check what the channel pins before and after with
`greentic-dev release view --tag dev`.
