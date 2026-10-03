# Contributing

Use Rust stable and Node 22.12+. Validate each change with focused local checks covering the affected behavior before committing. Use the full local suite for broad changes and release preparation. Keep changes narrow and complete: working behavior, a focused test, honest compatibility documentation, then a commit.

Development uses `main`; do not force-push or rewrite another contributor's history. External contributions may use pull requests. Avoid new dependencies where existing libraries/native platform features suffice.

Database integration tests must use a disposable local instance. Never include credentials, user database dumps or secret-bearing URLs in tests, screenshots, issues or logs.

Do not inspect, copy, translate or derive code from Beekeeper's commercially licensed directories. Independently implement behavior. If GPL community source is reused, preserve its license obligations and record the contribution before merging; the MIT license of original Klyndb code does not override third-party licenses.

See ARCHITECTURE.md for the driver contract. Add a real-server test and update docs/COMPATIBILITY.md for new database engines. Do not expose a driver as supported because it only connects.

Update `ROADMAP.md` in every meaningful implementation commit with completed behavior, validation and the next unfinished milestone. Keep its evidence consistent with `docs/VALIDATION.md`.

GitHub Actions is disabled by owner instruction. Use `./scripts/check.sh` for the full local suite; routine commits need focused checks, not every engine and platform test. Do not re-enable Actions or add automated workflow triggers without a new explicit owner instruction. Configure real integration test URLs only for disposable databases.
