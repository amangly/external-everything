# Palepale reference flow

## Provenance and scope

Reference: [amangly/palepale at `6490070271354e2a13a30e107065338c530a7322`](<https://github.com/amangly/palepale/tree/6490070271354e2a13a30e107065338c530a7322>). The launcher boundary and checked process-metadata approach are adapted using hyper-reV's own ABI. **The shared game renderer/menu implementation is directly copied from this pin:** Direct2D/DirectWrite overlay, D3D11/ImGui renderer, Chrome menu shell, and theme. GPUI stays limited to the loader/game chooser. Licensing, update delivery, spoofing, injection, and capture exclusion are excluded.

**PointBlank provenance is separate:** the pinned Palepale game modules exclude PointBlank. The [new reader](<../crates/external-runtime/src/games/pointblank/README.md>) derives from the supplied [local Rust reader](<C:/Users/Administrator/Downloads/pointblank-cheat/rust_external/src/game.rs>), [legacy verified ledger](<C:/Users/Administrator/Downloads/pointblank-cheat/internal/offsets_verified.h#L5-L60>), and typed frame/geometry evidence. Its current profile adds fresh local RTTI and direct-hypercall field proofs, rather than copying the legacy RVAs to a changed image. The supplied [PBZPTID legacy header](<C:/Users/Administrator/Documents/PBZPTID/Dll/Offsets.h>) contributes zero typed runtime offsets; its byte-patch patterns are unused.

The [workspace][r-workspace] contains the Tauri desktop, loader library, hvcall transport, and separate `df-external` runtime. The loader [manifest][r-loader] declares a library, so its executable entry is the [desktop main][r-main], rather than a loader main module. Its [library map][r-modules] groups setup, assets, process discovery, preflight, contracts, and other product-specific modules.

## Reference lifecycle

```text
Tauri executable startup -> isolated health probe -> snapshot / startup update
  -> Home containing game selection AND Setup controls
  -> preflight -> confirmation -> BootSetup -> restart requested
  -> reopen launcher -> live attachment + selected game process
  -> external compatibility probe -> external child -> launcher exits
```

| Stage | Source-backed behavior |
| --- | --- |
| Startup | [Desktop guard][r-guard] runs `--internal-hv-probe` with a three-second deadline; failure disables calls in the parent. A later live predicate determines attachment availability. |
| UI state | [React state][r-state] uses `booted`, `busy`, `page`, and `confirm`. `booted` tracks startup update completion, rather than attachment readiness. `boot-update` carries `{done,total}`. |
| Home / selection | [Home][r-home] shows the licensed game picker before attachment is live; PLAY is disabled until live and the selected game is present. This is a combined Home, rather than a strict setup-first wizard. |
| BootSetup | [Setup][r-setup] applies prerequisite gates, resolves assets, stages/promotes boot files, verifies the installation path, and records installed state. The GUI invokes setup with restart enabled. |
| Restart required | [Setup completion][r-restart] requests a restart after success. Restart-command errors are discarded; returned setup success means installation completed and restart was requested. |
| Live attachment | [Snapshot][r-snapshot] reads the live predicate independently of installed metadata. [hvcall liveness][r-live] accepts a nonzero usable CR3 range. It establishes response availability, rather than game compatibility or attachment build identity. |
| External verification | [Contract validation][r-contract] invokes `df-external.exe contract --json`, checks schema `1`, name, exact version, game support, and embedded game-data identifier. It uses a five-second timeout and bounded output. |
| External process | [Launch][r-launch] checks the contract, attachment, and selected game process, then spawns `df-external.exe --game <id>` from its directory. Startup acknowledgement is product-dependent. |
| Handoff | Successful [PLAY][r-play] exits the launcher. Source lifecycle provides startup handoff, rather than a persistent parent supervising return to a chooser. |

### Child protocol and startup failures

The [contract producer][r-contract-producer] returns:

```json
{"schema":1,"name":"df-external","version":"…","games":[{"id":"…","dump":"…","live_check":true}]}
```

The [child parser][r-child-parser] accepts `--game <id>`, `--game=<id>`, and an optional positional mode. Its default mode dispatches the selected game's runtime; product modes can differ, so a game entry alone does not imply an external popup.

For guarded products, [named Windows events][r-events] use `Local\PalepaleOverlay-<pid>-<nonce>-started` and `-ready`, passed as `PALEPALE_OVERLAY_STARTED_EVENT` and `PALEPALE_OVERLAY_READY_EVENT`. The [child entry][r-child-entry] signals started immediately; [the signal helper][r-signal] opens and sets the supplied event. Ready is backend-defined. The optional browser-radar branch adds `PALEPALE_RADAR_READY_EVENT`.

[Launch waits][r-waits] allow eight seconds for start and twenty seconds for Apex/Finals readiness, or eighty seconds for the other guarded branch. Other products return after spawn. General failures kill/reap a still-running child and keep the launcher open. [Apex exceptions][r-launch-end] treat duplicate exit code `3`, or a still-running reader after ready timeout, as a successful handoff. These acknowledgements are one-way startup signals, rather than an ongoing runtime-control protocol.

END or the runtime's [QUIT control][r-quit] requests shutdown. [Apex cleanup][r-cleanup] tears down presentation, stops workers, and joins them. The already-exited launcher is not respawned by that path. [Restore][r-restore] verifies the stock boot-manager path before cleanup and only reaches restart scheduling after successful preceding work. Partial-operation failures remain errors rather than successful removal.

## External Everything adaptation

Shared renderer provenance: [reference Overlay][r-overlay], [Gui][r-imgui], [Chrome][r-chrome], [theme][r-theme], and [key behavior][r-keys]. Local adaptations preserve drawing/menu APIs while changing branding/storage, owned teardown, capture-visible surfaces, and a polled INSERT/END worker. Game memory and offsets stay in the existing PointBlank engine/profile modules.

| Boundary | Current application behavior |
| --- | --- |
| UI ownership | [Application GPUI UI](<../apps/desktop/src/ui.rs>) owns loader and real chooser windows; hyper-reV supplies its backend rather than its window. |
| Ordering | Checking → Setup or Restart required → **Ready attachment** → Games. Installed state and catalog metadata never bypass readiness. |
| Setup / Restore | [Backend](<../apps/desktop/src/backend.rs>) delegates to the existing hyper-reV loader and retains manual restart. |
| Attachment | [Isolated probe](<../apps/desktop/src/probe.rs>) uses `--internal-attachment-probe`, a three-second deadline, and schema-1 JSON. [Transport](<../crates/hyperrev-transport/src/lib.rs>) requires the exact vendor test response and usable CR3. Palepale's register protocol is not copied. |
| Catalog | The source [catalog](<../games/catalog.json>) registers PointBlank by ID/name. [Build](<../tools/build.ps1>) packages the built executable and generates its path/SHA-256; [validation](<../apps/desktop/src/catalog.rs>) checks relative paths and x64 PE shape. |
| Child contract | [Contract validation](<../crates/external-runtime/src/contract.rs>) checks schema `1`, `external-runtime` identity, exact workspace version, backend ABI, and selected-game metadata. The child receives `--game pointblank`; matching argument spelling alone does not establish `df-external` binary compatibility. |
| Runtime ownership | `RuntimeSession` retains the owned child; the UI polls exit, refocuses its still-open chooser, and asynchronously rechecks readiness/catalog state. Lost readiness returns to the loader; exit errors survive in details. Dropping a running session stops/reaps it. |
| Acknowledgement | [Startup coordination](<../crates/external-runtime/src/launch.rs>) waits eight seconds for started, then twenty for renderer-ready. Events use `EXTERNAL_EVERYTHING_STARTED_EVENT` and `EXTERNAL_EVERYTHING_READY_EVENT`, with `Local\ExternalEverything-runtime-` names. Ready follows successful initialization/first frames of both presentation surfaces, separately from profile admission or a match. Bidirectional runtime IPC remains separate work. |
| Memory/process boundary | [Transport](<../crates/hyperrev-transport/src/lib.rs>) implements full-width direct virtual-copy reads/writes. [Process discovery](<../crates/hyperrev-transport/src/process.rs>) validates kernel-derived fields, current/System identity, reciprocal links, selected PID, CR3, and PE headers. Palepale's register ABI is excluded. |
| PointBlank presentation | [Copied common modules](<../crates/external-runtime/src/common/mod.rs>) reuse Palepale [Direct2D/DirectWrite Overlay](<../crates/external-runtime/src/common/overlay.rs>), [D3D11/ImGui Gui](<../crates/external-runtime/src/common/imgui_ui.rs>), and [Chrome](<../crates/external-runtime/src/common/chrome.rs>)/theme. PointBlank supplies its ESP adapter, ESP/MISC/CFG tabs, settings, engine worker, and reports. INSERT toggles the menu; END exits. Capture-visible surfaces replace capture-affinity paths. |
| Excluded product services | License/authentication, remote asset synchronization, self-update, hardware spoofing, injection, and capture exclusion are intentionally omitted. |

**Current profile admission:** the legacy November 15 profile (`edeeeef3…`, `0x01f6f000`) remains intact; the current November 16 profile (`d7de2e99…`, `0x01f7c000`, product `127.1.2611.16`) is now registered after [RTTI identity proof](<../build/pointblank-live-prefault-b92e786000b848bbb91947a31c2fc7fe/core-rtti.json>) and [fresh hypercall field-chain validation](<../build/pointblank-live-prefault-b92e786000b848bbb91947a31c2fc7fe/chain-proof-c14a95ee91374233b588c6881b9ac328.json>). That live proof establishes stable 2560×1080 camera matrices and one typed Avatar's health/transform. Engine selection requires the full exact game hash and profile image size, followed by typed runtime checks. Diagnostic captures or offline RTTI candidates alone never override admission.

**Live collection verified:** the bounded current-profile scan produced [FullCollection: 12 typed entries, 12 entities, 0 rejected](<../build/pointblank-collection-6e9ed0a515194a21a1002c2589b9f32d.json>), including a validated manager, float health above 100, and projected estimated rectangles. `SingleAvatar` remains a distinct fallback. Bones, teams, semantic foot anchoring, and exact overlay alignment remain open. The [runtime guide](<../crates/external-runtime/README.md#verified-profiles-and-remaining-limits>) records the exact hashes and separates collection proof from transport/window checks.

**Copied-renderer native checkpoint:** the eight-second harness produced 480 overlay frames, 206 menu frames, 139 live ESP frames, peak three painted projections, 14 latest entities, PID `10432`, and both Direct2D/D3D11 ready flags. Started/ready/child-cleanup checks and repeat 65-KiB transport/current-PID checks passed. Regression passed 164 tests (four separately ignored), Clippy, and Release publication. The final DPI-aware [report](<../build/renderer-verification-8a1e725916ec4e2baa9afa064de8c08c.json>) separately passed 478 overlay/menu frames and both ready flags in a waiting state: zero entities/ESP, rather than the earlier live counts. Final native handshake/cleanup and loader-caption checks passed. See [runtime details](<../crates/external-runtime/README.md#verified-profiles-and-remaining-limits>), [architecture](<architecture.md>), and [deployment](<../games/README.md>).

[r-workspace]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/Cargo.toml#L1-L17
[r-loader]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/loader/Cargo.toml#L7-L17
[r-main]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/apps/desktop/src-tauri/src/main.rs#L412-L455
[r-modules]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/loader/src/lib.rs#L4-L28
[r-guard]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/apps/desktop/src-tauri/src/main.rs#L8-L34
[r-state]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/apps/desktop/src/App.tsx#L917-L1100
[r-home]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/apps/desktop/src/App.tsx#L499-L595
[r-setup]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/loader/src/setup.rs#L1348-L1493
[r-restart]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/loader/src/setup.rs#L1494-L1510
[r-snapshot]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/loader/src/setup.rs#L1587-L1605
[r-live]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/hvcall/src/lib.rs#L366-L368
[r-contract]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/loader/src/overlay_contract.rs#L48-L138
[r-launch]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/loader/src/setup.rs#L1776-L1888
[r-play]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/apps/desktop/src-tauri/src/main.rs#L292-L312
[r-contract-producer]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/df/src/common/contract.rs#L71-L88
[r-child-parser]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/df/src/main.rs#L362-L424
[r-events]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/loader/src/setup.rs#L30-L122
[r-child-entry]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/df/src/main.rs#L88-L105
[r-signal]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/df/src/common/launch_signal.rs#L5-L24
[r-waits]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/loader/src/setup.rs#L1891-L1917
[r-launch-end]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/loader/src/setup.rs#L1931-L1968
[r-quit]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/df/src/common/chrome.rs#L774-L778
[r-cleanup]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/df/src/apex/mod.rs#L1552-L1561
[r-restore]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/loader/src/setup.rs#L1513-L1564
[r-overlay]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/df/src/common/overlay.rs#L1-L1214
[r-imgui]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/df/src/common/imgui_ui.rs#L1-L875
[r-chrome]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/df/src/common/chrome.rs#L1-L802
[r-theme]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/df/src/common/theme.rs
[r-keys]: https://github.com/amangly/palepale/blob/6490070271354e2a13a30e107065338c530a7322/crates/df/src/common/keys.rs#L1-L181
