# External Everything

Native Windows x64 application in Rust. The loader and game chooser use [GPUI](<https://gpui.rs/>) with Liquid Glass-inspired styling; the external ESP and cheat menu reuse Palepale's Direct2D/DirectWrite and D3D11/ImGui renderers.

**Current scope:** an application-owned launcher: **Setup → restart required → verified hyper-reV attachment → game chooser → separate external runtime**. The app reuses the pinned fork's **Setup / Restore backend**, direct hypercall memory transport, and checked process/CR3 discovery. PointBlank presentation directly reuses the [pinned Palepale shared renderer/menu implementation](<docs/reference-flow.md>), rather than a separate presentation implementation.

**PointBlank profiles:** exact-hash profiles cover the legacy November 15 image (`0x01f6f000`) and current November 16 image (`0x01f7c000`, product `127.1.2611.16`). Fresh hypercall reads verified the camera/Avatar chains and a live **FullCollection: 12 typed entries, 12 entities, 0 rejected**, with projected estimated boxes. **SingleAvatar** remains a separate fallback. Bones, teams, semantic foot anchoring, and exact box alignment remain unresolved; other hashes/sizes stay rejected. See the [profile evidence and runtime limits](<crates/external-runtime/README.md#verified-profiles-and-remaining-limits>).

## Checkout

Clone the [private application repository](<https://github.com/amangly/external-everything>) with an authenticated GitHub account:

```powershell
git clone --recurse-submodules https://github.com/amangly/external-everything.git
cd external-everything
```

For an existing checkout:

```powershell
git submodule update --init --recursive
```

[hyper-reV](<vendor/hyper-reV/README.md>) is a Git submodule sourced from the [maintained fork](<https://github.com/amangly/hyper-reV>). The parent repository pins an exact commit; the application uses its loader backend and the matching attachment ABI, rather than invoking its setup UI.

## Build

Use Windows x64, Rust 1.88 or later with the x64 MSVC toolchain, Visual Studio C++ build tools, and a Windows SDK. First build the standalone EFI image and architecture-matching attachment using the vendor's [build prerequisites](<vendor/hyper-reV/README.md#71-uefi-boot-compilation>), [architecture selection](<vendor/hyper-reV/README.md#72-architecture-specific-compilation>), and [standalone build guide](<vendor/hyper-reV/README.md#76-standalone-boot-and-debugger-checks>). The application build packages those existing native artifacts; it does not compile them.

The default payload directory is `vendor/hyper-reV/build/Release`, containing:

```text
uefi-boot-standalone.efi
hyperv-attachment.dll
```

Run the [build helper](<tools/build.ps1>) from the application repository root:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\build.ps1 -Publish
```

`-Action` accepts `Build`, `Check`, `Test`, and `Clippy`, defaulting to `Build`. `-Configuration` accepts `Debug` or `Release`, defaulting to `Release`. Build stages the following bundle under `dist/`; `-Publish` also copies it to `%LOCALAPPDATA%\External Everything\bin`:

```text
external-everything.exe
uefi-boot-standalone.efi
hyperv-attachment.dll
games/
  catalog.json
  pointblank/
    external-runtime.exe
```

The source [game catalog](<games/catalog.json>) registers PointBlank by ID/name. The helper packages the built runtime and generates its executable path and SHA-256 in the staged/published catalog. This runtime hash is separate from the accepted game-executable hash. See the [game runtime contract](<games/README.md>).

Use `-PayloadDirectory` to select another matching native build. A local smoke build may reuse artifacts from the original checkout:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\build.ps1 -PayloadDirectory C:\Users\Administrator\Documents\hyper-reV\build\Release
```

That machine-local path and its binaries stay outside Git. Checks use the same helper:

```powershell
cargo fmt --all --check
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\build.ps1 -Action Test
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\build.ps1 -Action Clippy
```

The [flow check](<tools/test-flow.ps1>) verifies the live startup route without boot writes. The [window smoke test](<tools/test-window.ps1>) exercises loader caption controls; pass `-ExpectedTitle 'External Everything · Games'` for the chooser. The [renderer harness](<tools/test-renderer.ps1>) runs an eight-second native ImGui/Direct2D smoke with owned startup events, a menu screenshot, frame report, and bounded shutdown. The final DPI-aware [renderer report](<build/renderer-verification-8a1e725916ec4e2baa9afa064de8c08c.json>) passed both ready flags with 478 overlay/menu frames in a waiting state (zero entities/ESP). An earlier live checkpoint separately recorded 14 entities and 139 ESP frames. Regression passed 164 tests and Clippy; Release publication and final native handshake/cleanup checks passed. See the [runtime verification details](<crates/external-runtime/README.md#verified-profiles-and-remaining-limits>) for scope and evidence. Startup inspection stays read-only; captures go under `build/screenshots/`.

## Run

Keep the published bundle together and open the application. Standard Windows UAC elevation precedes the app-owned launcher.

1. Startup runs an isolated attachment check. An inactive attachment routes to **Setup**, or **Restart Windows** when the recorded installation is present.
2. **Setup** delegates discovery, validation, journaling, and boot installation to hyper-reV. Restart Windows manually when ready; successful installation alone leaves the game chooser locked.
3. Reopen the app after booting through hyper-reV. Only a successful attachment test and usable guest CR3 open the separate **Games** chooser window.
4. Choose **PointBlank** in the published bundle. The launcher checks its runtime path, SHA-256, x64 PE, versioned contract, backend ABI, and fresh attachment state; it then waits for process-start and renderer-ready acknowledgements.
5. The external process uses Palepale's Direct2D/DirectWrite ESP and D3D11/ImGui menu shell. **INSERT** toggles the menu; **END** exits. PointBlank-specific ESP/MISC/CFG tabs use the existing shared controls and persistent settings. Game reads use the selected process CR3 through hyper-reV; unsupported hashes/image sizes stay rejected.
6. The chooser remains alive during an external session and regains focus when the child exits. The app rechecks attachment/catalog state before enabling another launch, returning to the loader if readiness is lost. Closing the launcher stops its owned running child.

Boot ownership stays with hyper-reV: firmware entries remain **hyper-reV** and **Safe Windows**, and the default recovery journal remains `%LOCALAPPDATA%\hyper-reV\boot-install.json`. **Restore** uses that journal; restart remains manual. Setup checks UEFI mode, Secure Boot off, and Hyper-V prerequisites. See the [vendor loader guide](<vendor/hyper-reV/loader/README.md>) for backend recovery and the separate vendor CLI.

The lifecycle follows the [pinned Palepale reference](<docs/reference-flow.md>), with stricter attachment-before-selection ordering. License sign-in, remote updates, and hardware spoofing are intentionally outside this application flow.

## Vendor updates

The [sync helper](<tools/sync-vendor.ps1>) fetches the fork and checks out the requested ref as a detached commit:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\sync-vendor.ps1 -Ref main
```

It prints the candidate change and leaves the submodule pointer for review and a parent-repository commit. Add `-Recurse` when rebuilding native payloads to sync the vendor's nested C++/UEFI dependencies too. The helper leaves remote branches unchanged. Review the loader API and payload compatibility, rebuild the vendor artifacts, then run application checks before accepting an update.

## Layout and contribution

- [Workspace manifest](<Cargo.toml>): shared Rust configuration.
- [Desktop package](<apps/desktop/Cargo.toml>) and [entry point](<apps/desktop/src/main.rs>): app-owned launcher and isolated probe entry.
- [UI](<apps/desktop/src/ui.rs>), [backend](<apps/desktop/src/backend.rs>), [probe](<apps/desktop/src/probe.rs>), and [catalog](<apps/desktop/src/catalog.rs>): lifecycle and external-process boundary.
- [Transport](<crates/hyperrev-transport/src/lib.rs>): pinned readiness, bounded virtual-memory read/write, and checked [process discovery](<crates/hyperrev-transport/src/process.rs>).
- [Architecture](<docs/architecture.md>) and [reference provenance](<docs/reference-flow.md>): implementation boundaries and source-backed flow comparison.
- [External runtime](<crates/external-runtime/README.md>) and [game runtime guide](<games/README.md>): PointBlank control/overlay, guarded reader, packaging, and startup protocol.
- [Vendored fork](<vendor/hyper-reV/README.md>): shared boot backend and native payloads.

Keep vendor changes in the fork and update the pinned submodule; keep application-specific work in this repository. Use Conventional Commits, such as `feat(desktop): ...`, `docs: ...`, and `chore(vendor): ...`. Licensed under GPL-3.0-only; retain the vendor's licensing and attribution.
