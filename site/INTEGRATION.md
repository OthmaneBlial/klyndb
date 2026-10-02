# Site maintenance

Portable static showcase for the canonical `OthmaneBlial/klyndb` repository. Publish this folder to `OthmaneBlial/OthmaneBlial.github.io/klyndb/`. That existing Pages repository currently publishes from `master`; preserve its configuration and other project folders. The application repository uses direct `main` commits and has GitHub Actions disabled.

Use relative local assets. The JPEGs are actual native macOS captures with synthetic validation data; the SVGs are original branding/illustrations. IBM Plex fonts retain their bundled OFL license in assets/FONT-LICENSE.txt. No telemetry, external JavaScript or runtime dependencies are included.

Run `node --check site/app.js`, serve over HTTP, verify desktop/mobile layout and snippet copy buttons, and check the published HTTPS page after pushing. Update public capabilities from the real app and ROADMAP.md.
