# Palepale reference flow

## Provenance and scope

Reference: [amangly/palepale at `6490070271354e2a13a30e107065338c530a7322`](<https://github.com/amangly/palepale/tree/6490070271354e2a13a30e107065338c530a7322>). This comparison uses the pinned source checkout, rather than a current-branch or live-game compatibility claim. It adapts the launcher/attachment/external-process boundary; game implementations, licensing, update delivery, and hardware spoofing are outside this port.

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

| Boundary | Current application behavior |
| --- | --- |
| UI ownership | [Application GPUI UI](<../apps/desktop/src/ui.rs>) owns loader and real chooser windows; hyper-reV supplies its backend rather than its window. |
| Ordering | Checking → Setup or Restart required → **Ready attachment** → Games. Installed state and catalog metadata never bypass readiness. |
| Setup / Restore | [Backend](<../apps/desktop/src/backend.rs>) delegates to the existing hyper-reV loader and retains manual restart. |
| Attachment | [Isolated probe](<../apps/desktop/src/probe.rs>) uses `--internal-attachment-probe`, a three-second deadline, and schema-1 JSON. [Transport](<../crates/hyperrev-transport/src/lib.rs>) requires the exact vendor test response and usable CR3. Palepale's register protocol is not copied. |
| Catalog | [Initial catalog](<../games/catalog.json>) is empty. [Validation](<../apps/desktop/src/catalog.rs>) supports pending entries and checked relative executable paths, SHA-256, and x64 PE shape. Concrete game backends remain later work. |
| Child contract | The verified executable receives `--game <game-id>`, matching the reference argument spelling for future backend reuse. That spelling alone establishes zero binary/runtime compatibility with `df-external`. |
| Runtime ownership | `RuntimeSession` retains the child handle; the UI polls exit, refocuses its still-open chooser, and asynchronously rechecks readiness/catalog state before another launch. Lost readiness returns to the loader; child-exit errors survive in details. Dropping a running session stops/reaps its child. Launch also rechecks readiness. |
| Acknowledgement | Current launch establishes successful process creation only. Game/window readiness acknowledgement and bidirectional IPC remain future integration work. |
| Excluded product services | License/authentication, remote asset synchronization, self-update, and hardware spoofing are intentionally omitted. Compatibility metadata belongs with future backend descriptors, independently of those services. |

See [architecture](<architecture.md>) for component ownership and [game runtimes](<../games/README.md>) for the actual catalog and child CLI. Setup or probe success makes zero claims about an unimplemented game backend.

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
