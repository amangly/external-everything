# External Everything

Native Windows x64 application in Rust and [GPUI](<https://gpui.rs/>), with a minimal Liquid Glass-inspired interface and custom minimize, maximize/restore, and close controls.

**Current scope:** a separate application repository and pinned hyper-reV foundation. The desktop app reuses the fork's existing **Setup / Restore** boot-setup interface. External game tooling, hypercall memory read/write integration, and game-specific features remain future work.

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

[hyper-reV](<vendor/hyper-reV/README.md>) is a Git submodule at `vendor/hyper-reV`, sourced from the [maintained fork](<https://github.com/amangly/hyper-reV>). The parent repository pins an exact commit containing the shared `run_with_title` API; tracking the fork's branch alone leaves builds unreproducible.

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
```

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

The [window smoke test](<tools/test-window.ps1>) exercises native caption hit areas, minimize, maximize/restore, and close on an interactive Windows desktop. Startup inspection stays read-only, and captures go under `build/screenshots/`.

## Run

Keep the three bundle files together and launch `external-everything.exe`. The shared loader requests standard Windows UAC elevation and inspects status on startup. **Setup** performs the existing vendor discovery, validation, journaling, and boot installation; **Restore** uses that journal. Restart remains manual.

Branding changes the window title, not the vendor's boot ownership: firmware entries remain **hyper-reV** and **Safe Windows**, and the default recovery journal remains `%LOCALAPPDATA%\hyper-reV\boot-install.json`. Setup checks UEFI mode, Secure Boot off, and Hyper-V prerequisites. Full behavior, recovery, and CLI details are in the [vendor loader guide](<vendor/hyper-reV/loader/README.md>); the separate vendor CLI is outside this app's three-file bundle.

## Vendor updates

The [sync helper](<tools/sync-vendor.ps1>) fetches the fork and checks out the requested ref as a detached commit:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\sync-vendor.ps1 -Ref main
```

It prints the candidate change and leaves the submodule pointer for review and a parent-repository commit. Add `-Recurse` when rebuilding native payloads to sync the vendor's nested C++/UEFI dependencies too. The helper leaves remote branches unchanged. Review the loader API and payload compatibility, rebuild the vendor artifacts, then run application checks before accepting an update.

## Layout and contribution

- [Workspace manifest](<Cargo.toml>): shared Rust configuration.
- [Desktop package](<apps/desktop/Cargo.toml>) and [entry point](<apps/desktop/src/main.rs>): application branding and shared loader invocation.
- [Architecture](<docs/architecture.md>): current boundaries and planned transport.
- [Vendored fork](<vendor/hyper-reV/README.md>): shared GPUI setup UI, boot backend, and native payloads.

Keep vendor changes in the fork and update the pinned submodule; keep application-specific work in this repository. Use Conventional Commits, such as `feat(desktop): ...`, `docs: ...`, and `chore(vendor): ...`. Licensed under GPL-3.0-only; retain the vendor's licensing and attribution.
