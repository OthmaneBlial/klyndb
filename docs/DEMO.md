# Klyndb preview walkthrough

[Watch the 46-second video](https://othmaneblial.github.io/klyndb/#demo) · [Download MP4](https://othmaneblial.github.io/klyndb/assets/klyndb-preview-demo.mp4)

This silent, captioned **screenshot walkthrough** was assembled on 2026-10-03. It is not a continuous desktop screen recording and does not establish desktop acceptance of the Redis driver.

| Time | Scene | What is shown |
| --- | --- | --- |
| 00:00–00:04 | Your databases. Your rules. | Original Klyndb title card. Free, open-source alternative to DBeaver. |
| 00:04–00:14 | SQL tabs. Real results. Your workspace. | Existing actual native macOS screenshot: SQLite and MySQL connections, SQL tabs and a 10,000-row SQLite result with synthetic records. |
| 00:14–00:24 | Explore Redis keys, types and TTLs. | Production `KeyValueWorkspace` React component displaying catalog/hash values captured from the disposable Redis 7.4.11 integration fixture. |
| 00:24–00:31 | Inspect the original bytes. | The same component displaying the captured binary string exactly as `0x6100ff`. |
| 00:31–00:40 | Review production writes before they run. | Production-write confirmation and exact JSON argument array; canceled without execution. |
| 00:40–00:46 | Eight engines. One workspace. | Current source-build engines, downloadable Preview 1's four-engine scope and the project star link. |

Redis scenes use **simulated Tauri IPC** for presentation, including the scan cursor and command classification. Values come from actual disposable-server contract artifacts. Backend integration separately verifies native commands, six value types, cursor paging, ACL/read-only and production guards, verified TLS/mTLS and bounded replies; see [validation evidence](VALIDATION.md) and [Redis workflow](REDIS.md). No live Redis command reply is presented as a successful native desktop write.

The native screenshot is [workbench-macos.jpg](assets/workbench-macos.jpg). Original title/closing cards use the existing local IBM Plex font assets. New browser frames were captured through CUA; the raw frames and FFmpeg build recipe remain in ignored `artifacts/demo-2026-10-03/`. Original branding and synthetic records only; no credentials or user databases are included.

Delivery: H.264/MP4, 1920 × 1080, 30 fps, YUV 4:2:0, fast-start metadata, 46 seconds, no audio. The website provides native video controls, inline mobile playback, a downloadable MP4, a poster and English WebVTT captions. README links to the website player because portable Markdown video rendering varies.

A fresh continuous native-app recording remains pending until native window capture is available. This walkthrough keeps the present evidence visible without claiming broader platform or release readiness.
