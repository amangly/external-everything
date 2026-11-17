# PointBlank direct startup

The November 16 profile resolves the player manager through two module-owned value wrappers. It skips region enumeration and heap scans, including while waiting in the lobby.

```text
A = read32(base + 0x012670C4)
B = read32(base + 0x012670D8)
manager = read32(A).wrapping_add(read32(B))
```

The wrapper objects start at RVAs `0x012670B8` and `0x012670CC`. Each supported wrapper has `[0, 0, 1, payload_pointer, 0]`. The native initializer stores a seed and `manager - seed`; its accessor adds the two values. The observed callback tags select the four-byte copy path.

The [resolver](<../crates/external-runtime/src/games/pointblank/reader.rs>) checks the selected image, wrapper states, payload ranges, manager type, implementation backlink, vector bounds, unique typed entries, and two stable collection reads. It rereads both wrappers and payloads before accepting the result. An empty collection is valid. Uninitialized, changing, or unsupported wrapper states return a waiting result and retry on the next poll.

This layout is registered only for the exact November 16 hash in the [profiles](<../crates/external-runtime/src/games/pointblank/offsets.rs>). The legacy profile retains bounded discovery.

## Evidence

The same chain resolves two process lifetimes:

| PID | Payload A | Payload B | Manager |
| --- | --- | --- | --- |
| 10432 | `0x6AF` | `0x128604F9` | `0x12860BA8` |
| 10116 | `0x6625` | `0x15A3E00B` | `0x15A44630` |

The [runtime proof](<../build/direct-manager-runtime/verified-direct-chain.json>) links the historical 12-player collection to three current typed/backlinked reads. The [static trace](<../build/direct-manager-static/verified-manager-accessor.json>) follows construction, initialization, and the accessor; nine selected code spans match across the captures.

Five independently launched [first-poll captures](<../build/direct-start-benchmark-3033a24879e14e1980438606f6390341.json>) each returned 10 players, zero rejected, and eight projections, without scanning. Total invocation times were 43.13–51.21 ms (46.31 ms mean).

The final [five-second native run](<../build/pointblank-performance-cc61a88c2cb049a08886c65fb6bc0546.json>) painted 478 ESP frames, with up to nine projected entities. The first full collection arrived 225.87 ms after presentation entry; the first ESP draw followed at 248.83 ms. Reader rate was 119.58 updates/s, and displayed sample age was 4.06 ms mean / 7.43 ms p95.

A separate [eight-second render check](<../build/renderer-verification-81d36503d86f4e058f06e7d34035c04b.json>) painted 692 ESP frames and reached its first ESP draw at 286.75 ms. Both native renderers, launcher handoff, hypercall buffer tests, and child cleanup passed. Workspace/vendor regression: 201 passed, four environment-specific tests ignored; Clippy and Release publication passed.

Generated captures remain machine-local. The resolver, guards, profiles, and fixtures are versioned.
