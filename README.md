# External Everything

Native Windows x64 application in Rust and [GPUI](<https://gpui.rs/>), with a minimal Liquid Glass-inspired interface and custom minimize, maximize/restore, and close controls.

**Current scope:** an application-owned launcher: **Setup → restart required → verified hyper-reV attachment → game chooser → separate external runtime**. The app owns its GPUI windows and lifecycle while reusing the pinned fork's **Setup / Restore backend**. Attachment probing and catalog-based process supervision are implemented; the shipped game catalog is empty. Game-specific external windows, features, and memory read/write APIs remain later work.

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
```

The helper copies the [game catalog](<games/catalog.json>), whose initial `games` array is empty. It packages the application and two existing boot payloads; future game executables require their own reviewed integration and deployment. See the [game runtime contract](<games/README.md>).

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

The [flow check](<tools/test-flow.ps1>) verifies the live startup route without boot writes. The [window smoke test](<tools/test-window.ps1>) exercises native caption hit areas, minimize, maximize/restore, and close on an interactive Windows desktop; pass `-ExpectedTitle 'External Everything · Games'` to verify the attachment-gated popup. Startup inspection stays read-only, and captures go under `build/screenshots/`.

## Run

Keep the published bundle together and open the application. Standard Windows UAC elevation precedes the app-owned launcher.

1. Startup runs an isolated attachment check. An inactive attachment routes to **Setup**, or **Restart Windows** when the recorded installation is present.
2. **Setup** delegates discovery, validation, journaling, and boot installation to hyper-reV. Restart Windows manually when ready; successful installation alone leaves the game chooser locked.
3. Reopen the app after booting through hyper-reV. Only a successful attachment test and usable guest CR3 open the separate **Games** chooser window.
4. The current empty catalog displays a pending-backends state. A future registered runtime must pass path, SHA-256, and x64 PE checks, plus fresh attachment checks, before the app starts its separate process.
5. The chooser remains alive during an external session and regains focus when the child exits. The app rechecks attachment/catalog state before enabling another launch, returning to the loader if readiness is lost. Closing the launcher stops its owned running child.

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
- [Transport](<crates/hyperrev-transport/src/lib.rs>): pinned attachment readiness queries; memory APIs remain later work.
- [Architecture](<docs/architecture.md>) and [reference provenance](<docs/reference-flow.md>): implementation boundaries and source-backed flow comparison.
- [External runtime source home](<crates/external-runtime/README.md>) and [game runtime guide](<games/README.md>): the next per-game port, packaging, and child CLI contract.
- [Vendored fork](<vendor/hyper-reV/README.md>): shared boot backend and native payloads.

Keep vendor changes in the fork and update the pinned submodule; keep application-specific work in this repository. Use Conventional Commits, such as `feat(desktop): ...`, `docs: ...`, and `chore(vendor): ...`. Licensed under GPL-3.0-only; retain the vendor's licensing and attribution.
