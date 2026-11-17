# External runtime

The separate PointBlank game process uses **Palepale's copied ESP renderer and ImGui menu**, while GPUI remains limited to the [desktop loader/game chooser](<../../apps/desktop/src/ui.rs>). The [runtime manifest](<Cargo.toml>) uses ImGui 0.12 and Windows 0.58 graphics APIs; integration's locked normal-dependency tree confirms zero GPUI dependencies in this crate. Boot installation stays in the [vendor backend](<../../vendor/hyper-reV/loader/README.md>); routing and child ownership stay in the [launcher](<../../apps/desktop/src/backend.rs>).

## Reused presentation

The [common module](<src/common/mod.rs>) records source pin [Palepale `6490070271354e2a13a30e107065338c530a7322`](<https://github.com/amangly/palepale/tree/6490070271354e2a13a30e107065338c530a7322/crates/df/src/common>):

- [Overlay](<src/common/overlay.rs>): original Direct2D/DirectWrite ESP drawing and viewport APIs.
- [Gui](<src/common/imgui_ui.rs>): original D3D11/ImGui rendering, device/swap-chain handling, and menu surface.
- [Chrome](<src/common/chrome.rs>) and [theme](<src/common/theme.rs>): shared shell, tabs, widgets, configuration controls, and styling.
- [Keys](<src/common/keys.rs>): INSERT menu toggle and END exit through an owned, joinable polling worker; transport/concealment calls and global keyboard hooks are omitted.

The [PointBlank presentation](<src/presentation/mod.rs>) adapts the reference game shell around the unchanged engine/reader. [ESP](<src/presentation/esp.rs>) uses the copied drawing primitives, aspect-preserving viewport fitting, clipped estimated rectangles, float-health labels, and optional four-segment crosshair. [Menu](<src/presentation/menu.rs>) uses the existing Chrome shell with **ESP / MISC / CFG** tabs, rather than a replacement menu implementation. Unsupported aim, bones, and team controls are absent.

[Settings](<src/presentation/settings.rs>) persist master ESP, boxes, health, crosshair, and box color beneath `%LOCALAPPDATA%\External Everything\pointblank`. The main file is `settings.json`, with `settings1.json` through `settings3.json` for slots. Changes save through the shared menu; loading settings opens the menu. The integration uses capture-visible surfaces and excludes capture-affinity/screenshot-monitor paths, injection, licensing, remote updates, and spoofing.

## Verified profiles and remaining limits

The [engine](<src/engine.rs>) admits only full exact hashes from immutable [build profiles](<src/games/pointblank/offsets.rs#L46-L95>), requires their exact remote PE32 image size, then applies typed runtime guards:

| Profile | Game SHA-256 | SizeOfImage |
| --- | --- | --- |
| `pb_id_20261115` | `edeeeef3e76caa41b13c0a66762da95b90e989f09b8dff7d8480a9bd302ee084` | `0x01f6f000` |
| `pb_id_20261116` | `d7de2e99c257164ce8022ecf81dd1ea5fe1fa7e9c6ae8bba58bb78c4cb459b46` | `0x01f7c000` |

The current image, product `127.1.2611.16`, was admitted after [RTTI identity evidence](<../../build/pointblank-live-prefault-b92e786000b848bbb91947a31c2fc7fe/core-rtti.json>) and [fresh hypercall field-chain proof](<../../build/pointblank-live-prefault-b92e786000b848bbb91947a31c2fc7fe/chain-proof-c14a95ee91374233b588c6881b9ac328.json>). Those local generated files record stable 2560×1080 camera matrices and one typed Avatar's health/transform; offline RTTI candidates alone never admit a build.

A subsequent bounded scan produced [live FullCollection evidence](<../../build/pointblank-collection-6e9ed0a515194a21a1002c2589b9f32d.json>): **12 typed entries, 12 entities, zero rejected**, PID `10432`, target CR3 `797462528`, manager `310774696`, viewport `2560×1080`. Float health values include `100`, `142`, `120`, `132`, and `105`; a projected rectangle is approximately `[2226.24, 555.00, 68.44, 190.11]` pixels. `SingleAvatar` remains a distinct fallback.

Bones, teams, semantic foot anchoring, and exact overlay alignment remain open. The 1.8-world-unit standing height is a presentation estimate. Legacy reader APIs preserve their original defaults; profile-aware capture, validation, and discovery explicitly use the selected profile.

**Copied-renderer native checkpoint:** an eight-second run reported `frames=480`, `menu_frames=206`, `entities=14`, `esp_frames=139`, `projected_entities=3`, `game_pid=10432`, and both renderer-ready flags `true`. Started/ready/child-cleanup checks and repeat 65-KiB hypercall/current-process self-tests passed. Regression passed **164 tests** (28 app, 51 runtime, 31 transport, 54 vendor; four separately ignored), Clippy with warnings denied, and Release publication.

**Final DPI-aware verification:** the [report](<../../build/renderer-verification-8a1e725916ec4e2baa9afa064de8c08c.json>) records `frames=478`, `menu_frames=478`, both ready flags `true`, and zero entities/ESP/projections during that waiting state. The [final native menu screenshot](<../../build/screenshots/pointblank-palepale-menu-8a1e725916ec4e2baa9afa064de8c08c.png>) shows the complete copied green Chrome shell. The earlier 14-entity/139-ESP checkpoint remains separate evidence; those live counts never describe this final waiting-state run. Final integration again passed contract, 65-KiB self-test, PID discovery, started/ready/child cleanup, and loader-caption checks.

Published SHA-256 identities: runtime `42f24221bc531e5344b9cee7fc7c1a424767caf9c9eef94e82e26876b2b21687`; launcher `57dc26a3be9befd849aa2cb954cb2a170b9091fe2309129687506a5b39b4677f`. Exact presentation semantics remain open.

## Source layout and provenance

| Module | Responsibility |
| --- | --- |
| [Entry](<src/main.rs>) | CLI, startup signal, connection, presentation dispatch |
| [Contract](<src/contract.rs>) | Schema/version/backend identity and game metadata |
| [Launch](<src/launch.rs>) | Contract probe, named events, deadlines, owned child cleanup |
| [Engine](<src/engine.rs>) | Process identity, profile/hash selection, background frame reads |
| [Presentation](<src/presentation/mod.rs>) | Shared-renderer orchestration and `RunReport` |
| [Diagnostics](<src/diagnostic.rs>) | Selected-PID read evidence, gap/error records, separate from admission |
| [PointBlank reader](<src/games/pointblank/README.md>) | Typed x86 objects, geometry, collection consistency, frame states |

The game reader derives from the supplied [local Rust PointBlank reader](<C:/Users/Administrator/Downloads/pointblank-cheat/rust_external/src/game.rs>), [legacy verified ledger](<C:/Users/Administrator/Downloads/pointblank-cheat/internal/offsets_verified.h#L5-L60>), and typed frame/geometry guards, with fresh current-profile proofs. Palepale supplies the copied rendering/menu implementation and reference process-metadata approach; its pinned game modules exclude PointBlank. The supplied [PBZPTID legacy header](<C:/Users/Administrator/Documents/PBZPTID/Dll/Offsets.h>) contributes zero runtime offsets or byte patches. See [reference provenance](<../../docs/reference-flow.md>).

## Launch and readiness

```text
external-runtime.exe contract --json
external-runtime.exe --game pointblank
```

[Contract schema 1](<src/contract.rs>) reports `external-runtime`, exact workspace version, ABI `hyper-rev/cpuid-8a3b55c7927/v1`, and PointBlank metadata (`PointBlank.exe`, pointer width `32`, offset builds `pb-id-20261115,pb-id-20261116`). The launcher validates it within five seconds, in addition to runtime catalog hash/PE checks.

[Startup](<src/launch.rs>) passes named manual-reset events with prefix `Local\ExternalEverything-runtime-`:

- `EXTERNAL_EVERYTHING_STARTED_EVENT`: child entry, eight-second deadline.
- `EXTERNAL_EVERYTHING_READY_EVENT`: successful initialization/first frames of both presentation surfaces, twenty-second deadline after started.

[Native orchestration](<src/presentation/native.rs>) emits ready only after successful Direct2D begin/end and a D3D11/ImGui frame with a valid render target and clean paint status. The menu opens at startup even when game data is waiting/rejected. These presentation checks remain separate from accepted game data, match state, and precise ESP alignment. Startup failure/timeout drops the owned child and attempts stop/reap.

INSERT toggles the menu; END, the shared quit control, or native window close ends the runtime. An owned 100-ms engine worker is stopped, unparked, and joined; the key poll worker is joined, and the original renderer HWNDs are hidden/destroyed on drop. The launcher stays alive, polls completion, and rechecks attachment/catalog state on return. The events are one-way startup acknowledgements rather than bidirectional control IPC.

## Memory and diagnostics

[Transport](<../hyperrev-transport/src/lib.rs>) uses the fork's full-width CPUID ABI and explicit target CR3 for operation-3 virtual-copy reads/writes. Requests are limited to 1 MiB and split at both page boundaries. Local destinations receive volatile read/same-byte-write prefaults; sources receive volatile reads. Touching pages differs from pinning, and boolean-copy failure can leave partial effects. PointBlank game capture remains read-only.

[Process discovery](<../hyperrev-transport/src/process.rs>) derives supported fields from kernel exported instructions, verifies running/file kernel PE identity, checks System/current PID/CR3/PEB and reciprocal links, then validates target PID/PE headers. Windows handles provide path/region metadata; game bytes travel through hypercalls. Typed per-frame image/pointer/matrix/collection guards remain unchanged by the renderer migration.

```powershell
.\external-runtime.exe --self-test
.\external-runtime.exe --snapshot
.\external-runtime.exe --render-smoke 5
```

- `--self-test`: owned 32-byte and 65-KiB read/write/restore, own PID discovery and image-header check. These transport checks have native evidence; they avoid game writes.
- `--snapshot` / `--snapshot-scan <1..2000>`: guarded one-shot/bounded capture; only validated `FullCollection` ends a scan early.
- `--render-smoke <1..10 seconds>`: CLI and presentation enforce a ten-second limit. `RunReport` includes `frames`, `menu_frames`, `entities`, `esp_frames`, `projected_entities`, `game_pid`, `d3d11_ready`, and `d2d_ready`. `frames` includes transparent clears; `menu_frames` counts successful visible menus; `esp_frames` counts successful frames painting a live projected entity; `projected_entities` is the largest count painted in-session. Entity data comes from actual snapshots.
- [Diagnostic capture](<src/diagnostic.rs>) and [offline RTTI recovery](<../../tools/recover-pointblank-rtti.py>) preserve evidence without overriding profile admission.

The [renderer harness](<../../tools/test-renderer.ps1>) runs an eight-second smoke, owns started/ready events, finds the child's native ImGui HWND, captures its menu, checks successful render reports, and verifies bounded shutdown. The native checkpoint above passed those checks. Use the [build helper](<../../tools/build.ps1>) for workspace checks and the [catalog guide](<../../games/README.md>) for generated runtime-hash packaging. Tool hashes, accepted game hashes, data proofs, renderer smoke, and final published artifacts remain distinct verification stages.
