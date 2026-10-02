# Releases and local native candidates

## Preview 1 — macOS Apple Silicon

[Download v0.1.0-preview.1](https://github.com/OthmaneBlial/klyndb/releases/tag/v0.1.0-preview.1). The DMG and app ZIP contain version 0.1.0 from source `d105601341ecccba1d44ab07226402c43f28f1e2`. Copy the app to Applications while the existing app is closed. Connection metadata and workspace state live outside the bundle.

This is an ad-hoc signed development preview, without Developer ID signing, notarization or Gatekeeper approval. macOS may prevent opening it. Build from source if this distribution is unsuitable; no system security setting needs to be disabled. Native acceptance covers the exact ZIP-extracted app on macOS 26.6 arm64 with disposable SQLite data. Windows, Linux, Intel Macs and other macOS versions remain unverified.

The published assets include `SHA256SUMS`, the original `BUILD_INFO.json` and completed `NATIVE_ACCEPTANCE.json`. The latter records connection, SQL import, schema browsing, reviewed editing, CSV export, confirmed reconnect/rollback, cancellation/session reuse, disconnect and workspace restoration. The build manifest's original pending status is preserved; the later acceptance record supplies the completed native evidence. All five public asset downloads were independently compared with the accepted local bytes, and the remote tag points to the recorded source commit.

Download both packages and `SHA256SUMS` into one directory, then run:

```sh
shasum -a 256 -c SHA256SUMS
```

| Package | Bytes | SHA-256 |
| --- | ---: | --- |
| macOS arm64 app ZIP | 9,289,629 | `8820467f1b00d617a6c69bbf0b91cffe3bab9455ecb6f0d3691ae6f465cefed8` |
| macOS arm64 DMG | 11,583,432 | `12c0d73b5c44a314cf05206000c60680de60519934351a2f486d994ce5fab7ca` |

Read the release notes and [compatibility matrix](COMPATIBILITY.md) before using the preview. Additional engines, broader native coverage and full DBeaver parity remain on the roadmap.

## Build another candidate locally

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
