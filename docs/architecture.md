# Architecture

## Ownership

External Everything owns the Windows x64 GPUI loader/game chooser, attachment-check isolation, catalog, external runtime, and process supervision. **GPUI is launcher-only.** The external ESP and cheat menu directly reuse the pinned Palepale shared Direct2D/DirectWrite and D3D11/ImGui/Chrome implementation. The maintained hyper-reV fork owns boot installation and its native attachment; the app calls its backend rather than its setup UI.

```text
Application GPUI loader
  -> hyper-reV Setup / Restore -> firmware / ESP + recovery journal
  -> isolated attachment check -> hyperrev-transport -> attached hyper-reV
  -> Ready Games chooser -> runtime hash / PE / version / ABI contract
  -> owned PointBlank child -> started + first-renderer-frame acknowledgements
      -> background engine -> checked target CR3 -> typed frame reader
      -> copied Palepale Direct2D/DirectWrite ESP
      -> copied Palepale D3D11/ImGui/Chrome menu
  -> child exit -> readiness/catalog recheck -> chooser or loader
```

The [workspace](<../Cargo.toml>) contains the [desktop](<../apps/desktop/Cargo.toml>), [transport](<../crates/hyperrev-transport/Cargo.toml>), and [external-runtime](<../crates/external-runtime/Cargo.toml>) crates. The [desktop entry](<../apps/desktop/src/main.rs>) handles its internal probe before UAC elevation and starts the [app-owned UI](<../apps/desktop/src/ui.rs>).

## Launcher lifecycle

| Observation | Route |
| --- | --- |
| Probe pending | Checking |
| Attachment absent, installation absent | Setup |
| Attachment absent, recorded installation present | Restart required |
| Exact attachment test response and usable guest CR3 | Games chooser |
| Inspection/setup/probe failure | Error with details and retry |

Only attachment readiness opens Games. Setup success and catalog metadata independently grant zero launch capability. [Setup/Restore delegation](<../apps/desktop/src/backend.rs>) retains the vendor's validation, journal, ownership checks, and manual restart. A running attachment may remain responsive until the next boot even after Restore.

The UI opens a real replacement window before closing its predecessor. Background work leaves errors visible; a failed handoff preserves the existing window. The chooser stays alive while its child runs. Child exit triggers an asynchronous readiness/catalog recheck; absent readiness returns to the loader, and exit errors survive in details. Every later launch also checks readiness again.

### Isolated attachment probe

The [probe controller](<../apps/desktop/src/probe.rs>) starts the same executable with `--internal-attachment-probe`, accepts bounded schema-1 JSON, and enforces a three-second deadline. Crashes, timeouts, invalid results, or unusable CR3 values leave Games locked. [Transport classification](<../crates/hyperrev-transport/src/lib.rs>) checks the exact vendor test response and a nonempty CR3 physical-frame field. This establishes protocol readiness, rather than cryptographic attachment identity or game compatibility.

## Catalog, contract, and child ownership

The source [catalog](<../games/catalog.json>) registers PointBlank by ID/name. [Build](<../tools/build.ps1>) fills the published executable path and hash. [Catalog validation](<../apps/desktop/src/catalog.rs>) checks schema, unique IDs, relative paths, link rejection, bounded size, runtime SHA-256, and x64 PE shape.

[Launch](<../apps/desktop/src/backend.rs>) checks attachment, verifies the runtime, rechecks attachment, then calls the [runtime startup coordinator](<../crates/external-runtime/src/launch.rs>). That coordinator:

1. Runs `contract --json` with a five-second deadline and bounded output.
2. [Validates](<../crates/external-runtime/src/contract.rs>) schema `1`, identity `external-runtime`, exact workspace version, backend ABI `hyper-rev/cpuid-8a3b55c7927/v1`, and one selected game entry with pointer-width/build/process metadata.
3. Starts `external-runtime.exe --game pointblank` from its directory.
4. Waits up to eight seconds for `EXTERNAL_EVERYTHING_STARTED_EVENT`, then twenty seconds for `EXTERNAL_EVERYTHING_READY_EVENT`. Names use the `Local\ExternalEverything-runtime-` prefix with process/nonce/sequence suffixes.

Started acknowledges child entry. Ready is emitted after both external renderer surfaces initialize and their first frame succeeds, rather than after window creation alone. It remains separate from game presence, profile admission, entity coverage, and exact presentation semantics. Failure/timeout drops the owned child and attempts stop/reap. `RuntimeSession` retains `OwnedRuntime`; the launcher polls exit and keeps responsibility through shutdown. Bidirectional control IPC remains outside this startup-event contract.

## Direct memory and process transport

[Connection](<../crates/hyperrev-transport/src/lib.rs>) implements readiness and explicit bounded virtual-memory reads/writes using the fork's full-width CPUID ABI. Operation `3` copies between source and destination CR3/address pairs; it returns a boolean rather than a byte count. Requests are bounded to 1 MiB and split at both virtual-page boundaries. Failed transfers may have partial effects. Target CR3 stays explicit; the connection's own CR3 is never substituted for it.

[Process discovery](<../crates/hyperrev-transport/src/process.rs>) adapts the reference's metadata approach, rather than its hypercall register layout. [Windows kernel discovery](<../crates/hyperrev-transport/src/windows.rs>) reads the actual kernel's export leaf instructions for process-field offsets, checks running/file PE identity, validates System/current-process identity and reciprocal list links, then validates the selected PID's CR3 and remote MZ/PE. The remaining reference-derived structure hypotheses are verified at runtime. It performs metadata reads, rather than installing hooks or writing kernel fields.

Local copy buffers are prefaulted after range validation: read destinations receive volatile reads/same-byte writes to materialize demand-zero/COW pages; write sources receive volatile reads. This touches pages rather than pinning them, so later paging can still fail a copy. The change addresses the observed bulk-read failure on an initially unmaterialized local page.

`Connection::self_test` reads/writes/restores newly owned 32-byte and 65-KiB buffers. The runtime's `--self-test` additionally discovers its own PID and reads its own image header. Integration verified these native transport checks, separately from renderer initialization, game offsets, or live ESP alignment.

## PointBlank runtime and current build guard

The [runtime engine](<../crates/external-runtime/src/engine.rs>) selects PointBlank by process name/visible window, resolves its PID through the checked transport, selects an immutable [build profile](<../crates/external-runtime/src/games/pointblank/offsets.rs#L46-L95>) by full disk SHA-256, and requires the profile's exact remote PE32 image size before using its RVAs. Only the two registered profiles are admitted. Initial attachment uses checked kernel discovery, then retains an owned query/synchronization handle bound to PID and creation time. A zero-timeout wait checks that held process object's lifetime on steady-state polls, rather than repeating the kernel walk; exit/error clears the target. Game bytes travel directly through target-CR3 hypercalls; native handles provide identity/lifetime/path/region metadata.

The [typed reader](<../crates/external-runtime/src/games/pointblank/README.md>) comes from the supplied local Rust PointBlank source and verified ledger, supplemented by typed frame guards. It validates remote PE32 identity, typed camera/matrix/Avatar pointers, stable collections, and finite geometry. The dynamic manager scan is budgeted; a single observed Avatar stays explicitly distinct from a full collection. Standing-height box dimensions are presentation estimates; bones, teams, visibility, names, and exact box alignment remain unresolved.

Registered profiles retain the legacy `pb_id_20261115` image (`edeeeef3…`, size `0x01f6f000`) and add `pb_id_20261116` (`d7de2e99…`, size `0x01f7c000`, product `127.1.2611.16`). The current profile is backed by [captured RTTI identities](<../build/pointblank-live-prefault-b92e786000b848bbb91947a31c2fc7fe/core-rtti.json>) plus a separate [fresh field-chain proof](<../build/pointblank-live-prefault-b92e786000b848bbb91947a31c2fc7fe/chain-proof-c14a95ee91374233b588c6881b9ac328.json>): stable 2560×1080 camera matrices and one typed Avatar's float health/transform. Offline RTTI candidates alone never admit a profile.

A bounded current-profile scan subsequently resolved the live manager and produced [FullCollection evidence](<../build/pointblank-collection-6e9ed0a515194a21a1002c2589b9f32d.json>): PID `10432`, CR3 `797462528`, manager `310774696`, 12 typed entries, 12 entities, and zero rejected. Float health values include `142` and `132`; projected rectangles are present. `SingleAvatar` remains a separate fallback. Bones, teams, semantic foot anchoring, and exact screen alignment remain unresolved; the 1.8-world-unit box height remains an estimate. The profile-aware APIs preserve legacy defaults and fixtures. See the [runtime guide](<../crates/external-runtime/README.md#verified-profiles-and-remaining-limits>).

## Shared Palepale presentation

The [copied common modules](<../crates/external-runtime/src/common/mod.rs>) identify source pin `6490070271354e2a13a30e107065338c530a7322`. [Overlay](<../crates/external-runtime/src/common/overlay.rs>) retains the Direct2D/DirectWrite drawing API; [Gui](<../crates/external-runtime/src/common/imgui_ui.rs>) retains the D3D11/ImGui renderer; [Chrome](<../crates/external-runtime/src/common/chrome.rs>) and [theme](<../crates/external-runtime/src/common/theme.rs>) retain the shared menu shell/widgets. These are reused implementation files, rather than a new drawing/menu implementation.

[PointBlank presentation](<../crates/external-runtime/src/presentation/native.rs>) supplies an owned reader worker targeting 125 Hz through the shared high-resolution timer, replacing the former 100-ms/10-Hz cap; rendering follows monitor refresh capped at 120 Hz. The [eight-second native measurement](<../build/renderer-verification-02a8d2b8d98148b9873665b1532b41d1.json>) reached 124.30 reader polls/s and 98.82 rendered frames/s on the 100-Hz monitor; that sample was waiting with zero accepted data. It also supplies the [ESP adapter](<../crates/external-runtime/src/presentation/esp.rs>), [ESP/MISC/CFG tabs](<../crates/external-runtime/src/presentation/menu.rs>), [persistent settings](<../crates/external-runtime/src/presentation/settings.rs>), and `RunReport`. Settings and three slots live beneath `%LOCALAPPDATA%\External Everything\pointblank`. INSERT toggles the menu; END, shared quit, or native window close requests exit. Teardown stops/unparks/joins the engine worker, joins [key polling](<../crates/external-runtime/src/common/keys.rs>), and drops the native HWNDs. Capture-visible constructors are used; capture-affinity/screenshot-monitor paths, injection, licensing, remote update, and spoofing are excluded.

`--render-smoke <1..10 seconds>` uses the same real engine/render loop. It reports successful overlay/menu frames, latest entities/PID, renderer-ready flags, plus `esp_frames` and peak `projected_entities` to distinguish actual live-entity paints from transparent clears. CLI and presentation both enforce a ten-second limit. The shared menu is forced open until both first frames succeed, even while game data is waiting/rejected. The [renderer harness](<../tools/test-renderer.ps1>) exercises owned startup events, the native ImGui HWND, screenshot capture, eight-second rendering, and bounded shutdown. Its native checkpoint passed: 480 overlay frames, 206 menu frames, 139 ESP frames, 14 latest entities, peak three painted projections, and both renderer-ready flags.

### Low-latency capture policy

The current profile uses a [verified direct singleton](<pointblank-direct-startup.md>) on every poll, including while its collection is empty. It performs zero memory-region queries or heap scans. The [engine](<../crates/external-runtime/src/engine.rs>) retries attachment at 200 ms. The legacy profile retains 100-ms bounded discovery, 40-ms candidate probes, a 64-address cache, and five-second region refresh; scan-prefix progress survives budget stops. Attached readers check the held process lifetime rather than walking kernel metadata repeatedly.

[Live capture](<../crates/external-runtime/src/games/pointblank/reader.rs#L435-L599>) permits normal matrix movement within an unchanged typed camera chain/viewport, retains all image/pointer/vtable/collection/matrix-validity guards, and reprojects fresh world positions with the final validated camera. Strict `capture_profile` remains unchanged for diagnostic callers. [Frame publication](<../crates/external-runtime/src/presentation/timing.rs>) may hold a coherent same-PID frame for a consistency-change retry up to 24 ms; display data expires at 40 ms, while identity/read failures clear immediately.

`RunReport` now exposes measured reader/render rates, poll/accepted/transient counts, capture mean/max duration, elapsed time, and displayed-data age mean/p95/max. Targets remain distinct from achieved rates. The linked eight-second run measured 995 polls/791 renders while waiting. The durable [five-second performance report](<../build/pointblank-performance-7fe405138b524dd9be82b181882d9ecd.json>) measured 123.94 reader polls/s, 98.39 render frames/s, and 621 accepted SingleAvatar samples. Capture mean/max were 0.120/50.12 ms including startup; displayed-data age mean/p95/max were **4.38/8.01/8.36 ms**. One off-screen Avatar yielded zero ESP paints. These quantify capture/render cadence and sampled-data freshness, rather than game FPS, end-to-end input latency, or full-collection ESP performance.

## Vendor and packaging

The [submodule](<../.gitmodules>) pins the maintained [hyper-reV fork](<https://github.com/amangly/hyper-reV>). Shared boot/ABI changes belong there; app lifecycle and game logic belong here. Boot entries remain **hyper-reV** and **Safe Windows**, with recovery journal `%LOCALAPPDATA%\hyper-reV\boot-install.json`. The [vendor loader guide](<../vendor/hyper-reV/loader/README.md>) defines backend prerequisites and recovery.

The [build helper](<../tools/build.ps1>) builds the Rust workspace, validates and packages prebuilt matching boot payloads, and writes a generated runtime-hash catalog. Build stages the bundle under `dist/`; `-Publish` also targets `%LOCALAPPDATA%\External Everything\bin`:

```text
external-everything.exe
uefi-boot-standalone.efi
hyperv-attachment.dll
games/catalog.json
games/pointblank/external-runtime.exe
```

The two boot payloads remain executable-relative; native compilation is a separate vendor build. The generated runtime hash identifies the external tool, while the ledger hash identifies the accepted game image. The [vendor sync helper](<../tools/sync-vendor.ps1>) explicitly selects a candidate revision for review. Generated artifacts stay outside Git.

See [reference provenance](<reference-flow.md>), [runtime details](<../crates/external-runtime/README.md>), and [catalog deployment](<../games/README.md>). The copied renderer now has its own successful native smoke, started/ready/child-cleanup checks, and repeat 65-KiB transport/current-PID checks. The recorded checkpoint passed 164 tests (four separately ignored), Clippy with warnings denied, and Release publication. The final DPI-aware [report](<../build/renderer-verification-8a1e725916ec4e2baa9afa064de8c08c.json>) passed 478 overlay/menu frames with both ready flags, but zero entities/ESP in its waiting state; that is separate from the earlier live 14-entity/139-ESP run. Final native integration/cleanup and loader-caption checks also passed. The subsequent low-latency source passed 177 tests (four separately ignored), Clippy, and Release publication; its new reader/render rates and freshness measurements are recorded above, separately from these older collection/render checkpoints. See [current artifact identities](<../crates/external-runtime/README.md#verified-profiles-and-remaining-limits>); exact box alignment and unresolved game semantics remain distinct.
