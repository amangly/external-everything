# External Everything

The goal is to build external cheats for all games using hyper-reV's CPUID-based hypercalls. PointBlank is the first implemented game.

The Windows x64 launcher uses GPUI. Each game gets its own module, with shared memory transport, rendering, and menu code. The game runtime uses Direct2D for ESP and Direct3D11/ImGui for menus.

## Run

1. Open the launcher and choose **Setup** if needed.
2. Restart Windows through **hyper-reV**, then reopen the launcher.
3. Choose **PointBlank**. Start the game or join a match if the runtime is waiting.
4. Press **INSERT** to toggle the menu or **END** to exit the runtime.

PointBlank has boxes, health labels, a crosshair, and saved settings. Boxes use estimated body dimensions; team filtering and skeletons are unfinished. The reader checks the game build against its [profiles](<crates/external-runtime/src/games/pointblank/offsets.rs>).

**Restore** uses the boot journal. The firmware entries remain **hyper-reV** and **Safe Windows**. See the [boot setup guide](<vendor/hyper-reV/loader/README.md>) for prerequisites and recovery.

## Build

Requires Windows x64, Rust 1.88+, the MSVC C++ tools, and a Windows SDK.

```powershell
git clone --recurse-submodules https://github.com/amangly/external-everything.git
cd external-everything
```

Build the matching EFI image and attachment first using the [hyper-reV build guide](<vendor/hyper-reV/README.md>). Then package the application:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\build.ps1 -Publish
```

The helper reads payloads from `vendor/hyper-reV/build/Release`; use `-PayloadDirectory` for another build. It writes the bundle to `dist/` and publishes it to `%LOCALAPPDATA%\External Everything\bin`. Keep the bundle together.

## Checks

```powershell
cargo fmt --all --check
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\build.ps1 -Action Test
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\build.ps1 -Action Clippy
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\test-pointblank.ps1
```

The last command checks the installed runtime, hypercalls, rendering, and launcher handoff on an interactive Windows desktop.

## Code

- [Desktop](<apps/desktop/src/main.rs>): loader, game chooser, and runtime ownership.
- [Runtime](<crates/external-runtime/README.md>): PointBlank ESP and menu.
- [Transport](<crates/hyperrev-transport/src/lib.rs>): hyper-reV calls and process discovery.
- [Game package](<games/README.md>): catalog and binary layout.
- [Architecture](<docs/architecture.md>) and [reference](<docs/reference-flow.md>): implementation details.

Shared boot and attachment changes belong in the [hyper-reV fork](<https://github.com/amangly/hyper-reV>). Update the pinned vendor revision with the [sync helper](<tools/sync-vendor.ps1>). Licensed under GPL-3.0-only; retain upstream attribution.
