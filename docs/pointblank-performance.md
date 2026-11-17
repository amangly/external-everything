# PointBlank ESP latency

The current profile now uses a [verified direct singleton](<pointblank-direct-startup.md>), with zero heap scans. Five fresh first-poll captures returned the full 10-player collection in 43–51 ms; the native run painted its first ESP frame after 249 ms including presentation setup. The earlier reader-cadence measurements below describe the preceding latency fix.

## Causes in the first integration

- The capture worker slept **100 ms**, imposing a **10-Hz reader ceiling**, even though the native overlay rendered more often.
- Full process/CR3 discovery ran every ten polls. Increasing the reader rate alone would have multiplied those expensive kernel queries.
- The strict diagnostic reader required identical camera matrices before and after the entity batch. Normal camera movement could therefore discard otherwise valid live frames.

## Live-path changes

The [capture worker](<../crates/external-runtime/src/presentation/native.rs>) uses the copied reference's high-resolution waitable timer at **125 Hz**. The native overlay uses its monitor-aware render rate, capped at **120 Hz**, rather than a fixed 60-Hz cap.

The [engine](<../crates/external-runtime/src/engine.rs>) performs checked kernel discovery at attachment. An owned query/synchronize process handle then detects exit through a zero-time wait, retaining the original process identity across PID reuse. Retry, manager discovery, and region refresh use independent wall-clock cadences; steady-state capture performs neither full process-list walks nor region queries.

The [live reader](<../crates/external-runtime/src/games/pointblank/reader.rs>) permits valid camera matrix motion within the same typed camera chain and dimensions. It reprojects fresh entity positions with the final validated camera. Exact image/profile admission, typed objects, collection bounds/backlinks/uniqueness, and pointer consistency remain enforced. The original strict capture API and its diagnostic fixtures remain intact.

Discovery also caches up to 64 typed manager addresses while their collections are empty. It probes those addresses every 40 ms, so entering a match does not require a full heap rescan. The 100-ms scan keeps its cursor across transitions and budget stops. Invalid addresses leave the cache; the active collection still passes all per-frame checks.

A same-process consistency retry may retain the previous coherent sample for at most **24 ms**. The renderer clears data older than **40 ms**; identity/read failures clear immediately. These bounds avoid both one-frame flicker and indefinitely stale positions.

## Native measurement

The [five-second report](<../build/pointblank-performance-7fe405138b524dd9be82b181882d9ecd.json>) records:

| Measurement | Observed |
| --- | ---: |
| Reader polls / accepted samples | 621 / 621 |
| Reader rate | 123.94 updates/s |
| Native render rate | 98.39 frames/s |
| Mean capture cost | 0.120 ms |
| Mean published-sample age | 4.38 ms |
| Published-sample age, 95th percentile | 8.01 ms |
| Maximum published-sample age | 8.36 ms |

This run captured the single-Avatar fallback; the entity was outside projection, so it reports zero ESP-painted frames. It establishes reader cadence and sample freshness, rather than a full-collection performance claim or total input-to-display latency. The maximum capture cost, 50.12 ms, includes initial attachment; that startup work is outside steady-state sample ages.

Both native renderers, contract/launch acknowledgements, child cleanup, and process/hypercall self-tests passed again. A separate [native-close check](<../build/renderer-verification-52f73fc844c7441c8be120f4f01cdd4d.json>) exited cleanly with the capture worker stopped. That latency checkpoint passed 177 regression tests. The later manager-cache patch adds ten fixtures: **187 passed**, four environment-specific tests separately ignored. Clippy and Release publication passed.

The [renderer harness](<../tools/test-renderer.ps1>) and [PointBlank integration test](<../tools/test-pointblank.ps1>) save current performance reports and reject a full-duration run whose reader remains below 30 updates/s. Actual rates depend on the active game state and host workload.
