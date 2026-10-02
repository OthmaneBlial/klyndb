# Local native release candidates

GitHub Actions remains disabled. Run the configured local `scripts/check.sh` against disposable database fixtures before creating a candidate.

On macOS, from a clean application checkout:

```sh
bash scripts/package-macos-preview.sh
```

The script installs the locked frontend, regenerates the committed dependency notices, runs Rust security/license audits, builds the optimized native app and applies a local ad-hoc signature. It creates a compressed DMG and `.app.zip` under ignored `artifacts/release-candidates/`, alongside `BUILD_INFO.json` and `SHA256SUMS`. The manifest records the exact application source commit, architecture, builder macOS version and artifact hashes.

The DMG contains Klyndb.app and an Applications shortcut. Both native archives retain the project's MIT license, provenance record and locked dependency license texts. Packaging checks verify the architecture, signature, absence of development-machine library paths, read-only mounted DMG, executable/resource bytes, ZIP integrity and original notices. Check downloaded files with `shasum -a 256 -c SHA256SUMS` in their directory.

Ad-hoc signing does not establish Apple Gatekeeper approval. Developer ID signing and notarization need the owner's Apple credentials. The script does not disable any system security setting or publish a release.

Before publishing a preview, launch the actual packaged app and verify connection, schema browsing, SQL execution/cancellation, staged edits, native import/export and disconnect against disposable data. Retain independent database/export checks and the exact tested artifact hashes. A package passing archive checks alone is not native acceptance. The initial manifest deliberately records native acceptance as pending; attach completed validation evidence separately after the manual workflow passes.

Only advertise platforms and workflows actually checked. Current Windows/Linux/Intel-Mac package validation and the broader feature matrix remain pending. Public releases should identify unsupported engines, preview limitations, signing status and how to build from source; never tag an unverified candidate as a usable release.
