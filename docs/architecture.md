# Architecture

## Current foundation

External Everything is a separate Windows x64 Rust application. Its current feature is the existing hyper-reV **Setup / Restore** interface, presented with External Everything branding. Repository structure, dependency pinning, and reproducible packaging are the current milestone; external game features remain future work.

```text
External Everything desktop
  -> shared hyper-reV Rust loader
     -> GPUI Setup / Restore interface
     -> existing boot setup backend
        -> Windows firmware and ESP adapters
        -> standalone EFI image + architecture-matching attachment
```

### Application boundary

The [root workspace](<../Cargo.toml>) contains the [desktop package](<../apps/desktop/Cargo.toml>), named `external-everything`; its Windows executable is `external-everything.exe`.

The [entry point](<../apps/desktop/src/main.rs>) calls `hyperrev_loader::platform::elevate_gui_if_needed()` and then `hyperrev_loader::ui::run_with_title("External Everything")`. The vendor owns the native Rust/GPUI window, Liquid Glass-inspired styling, custom minimize/maximize/restore/close controls, startup inspection, and Setup / Restore behavior. Windows Acrylic supplies the glass material; high-contrast mode uses a solid surface. The application consumes that public API instead of copying or rewriting vendor implementation files.

Elevation follows the shared Windows UAC path. The window title and app identity are branded; boot entry names, ESP paths, recovery journal ownership, and setup semantics stay with hyper-reV. In particular, the default journal remains `%LOCALAPPDATA%\hyper-reV\boot-install.json`. See the [shared loader guide](<../vendor/hyper-reV/loader/README.md>) for setup, repeat/update handling, and recovery behavior.

### Vendor boundary

`vendor/hyper-reV` is a Git submodule sourced from [amangly/hyper-reV](<https://github.com/amangly/hyper-reV>), pinned by the parent repository to an exact commit that includes `run_with_title`. It remains an independently maintained project. Shared loader or native payload changes belong in that fork, followed by an explicit parent-repository submodule update.

The [vendor documentation](<../vendor/hyper-reV/README.md>) is authoritative for its EFI, attachment, usermode components, and native build requirements. This repository does not duplicate those sources or define a second boot setup implementation. The vendor debugger and CLI remain separate tools; the desktop bundle contains only the app and its two payloads.

### Build and packaging boundary

The [application build helper](<../tools/build.ps1>) builds or checks the root Cargo workspace. `Build` packages prebuilt `uefi-boot-standalone.efi` and `hyperv-attachment.dll` from `vendor/hyper-reV/build/Release` by default, with `-PayloadDirectory` available for another matching build. Build the vendor's standalone EFI image and architecture-specific DLL first, following its [standalone guide](<../vendor/hyper-reV/README.md#76-standalone-boot-and-debugger-checks>) and prerequisites.

The staged bundle lives under `dist/`; `-Publish` additionally copies it to `%LOCALAPPDATA%\External Everything\bin`. Both payloads sit beside `external-everything.exe`, matching the shared loader's executable-relative discovery. `Debug` and `Release` select the Rust configuration; the default payload source remains the vendor's Release directory unless overridden. Generated binaries and machine-local artifact paths stay outside Git.

The [vendor sync helper](<../tools/sync-vendor.ps1>) defaults to `-Ref main`: fetch the fork, resolve the ref, check out the resolved detached commit, and print the change for review. The resulting submodule pointer stays uncommitted until explicitly accepted. `-Recurse` also initializes the nested native dependencies after selecting the new fork revision. The helper leaves publishing and pushing to the maintainer; synchronization is an explicit action rather than an automatic tracking policy.

## Planned transport and external features

A future `crates/hyperrev-transport` crate is intended to isolate access to the fork's hypercall-based memory read/write capabilities. This is a planned boundary, not an implemented crate or a stable ABI contract.

```text
Future desktop features
  -> planned hyperrev-transport crate
     -> validated ABI of the pinned hyper-reV fork
        -> hypercall memory read/write
```

Before implementing that layer:

1. Validate the fork's actual ABI, supported operations, toolchain, and calling convention against the pinned source and native artifacts.
2. Keep protocol-specific and native interoperability details inside the transport layer.
3. Expose checked read/write operations with explicit results to application features, backed by tests of the validated integration.

Hypercall bindings, protocol adapters, toolchain integration, process/game integration, and external cheat features are future work. This foundation supplies neither an invented opcode/register layout nor a claimed working memory transport. Current setup success is separate from any future transport or feature validation.

## Change discipline

Use Conventional Commits. Review vendor API and payload compatibility when changing the pin; rebuild the native payloads and run the root Check/Test/Clippy actions before accepting it. Keep current setup functionality distinct from future transport work, preserve GPL-3.0-only licensing and upstream attribution, and keep generated artifacts outside version control.
