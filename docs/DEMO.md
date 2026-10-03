# 🎬 Klyndb in 64 seconds

[▶ Watch the demo](https://othmaneblial.github.io/klyndb/#demo) · [Download MP4](https://othmaneblial.github.io/klyndb/assets/klyndb-preview-demo.mp4)

A silent, captioned **screenshot walkthrough**, refreshed on 2026-10-03. **SQL, MongoDB and Redis scenes are actual native macOS captures**, with synthetic data and real database requests. This is an edited screenshot tour rather than a continuous screen recording.

| Time | Scene | What you see |
| --- | --- | --- |
| 00:00–00:04 | Your databases. Your rules. | Original Klyndb title card: free, open-source alternative to DBeaver. |
| 00:04–00:14 | SQL tabs. Real results. | Native macOS workspace with SQLite/MySQL connections and a 10,000-row SQLite result of synthetic records. |
| 00:14–00:20 | Filter MongoDB documents. | Native JSON filter and ordered sort against a real MongoDB 8.0.32 disposable database. |
| 00:20–00:26 | Aggregate documents. | Native grouping pipeline returns EU=102 and US=103 from 205 synthetic documents. |
| 00:26–00:32 | Review production writes. | Actual native production review with original BSON conflict protection and exact 64-bit integer text; canceled without submitting a write. |
| 00:32–00:42 | Explore Redis keys. | Actual native Redis 7.4.11 key explorer, synthetic hash values, types and TTLs. |
| 00:42–00:49 | Inspect original bytes. | Actual native binary string represented exactly as `0x6100ff`. |
| 00:49–00:58 | Review data commands. | Actual native production confirmation with captured JSON arguments; canceled without execution. |
| 00:58–01:04 | Nine engines. One workspace. | Source-build engines, downloadable Preview 1 scope and the GitHub star link. |

The MongoDB scenes come from the actual embedded `tauri://localhost` source debug app at `2cb5488`, with real native IPC. Separate native acceptance includes paging, indexes, insert/replace/delete, independent tabs and session invalidation; see [validation evidence](VALIDATION.md). They do not establish optimized-release or broader platform acceptance. The Redis scenes come from the actual embedded source debug app at `6c8f33d` and a separate restricted Redis 7.4.11 fixture. Native acceptance also verifies production Cancel/Confirm, read-only refusal and a delayed real reply across a tab switch. Updated public packages and other platforms remain pending.

The native images are [SQL workspace](assets/workbench-macos.jpg), [MongoDB aggregation](assets/mongodb-macos.jpg) and [Redis explorer](assets/redis-macos.jpg). All records are synthetic. No credentials or user databases appear. Original title/closing/poster cards reuse the local IBM Plex fonts. Raw CUA captures and FFmpeg recipes remain in ignored `artifacts/demo-2026-10-03/`, `artifacts/demo-native-mongodb/` and `artifacts/demo-native-redis/`.

Delivery: H.264/MP4, 1920 × 1080, 30 fps, YUV 4:2:0, fast-start metadata, 64 seconds, no audio. The website has native playback controls, mobile inline playback, English WebVTT captions, a poster and direct download. README links to the player because portable Markdown video rendering varies.

A continuous native recording, Redis SSH and broader platform/package acceptance remain on the roadmap.
