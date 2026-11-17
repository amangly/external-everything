# Single-file distribution

Distribute `external-everything.exe`. It contains the EFI image, attachment DLL, PointBlank runtime, and generated game catalog.

The [build helper](<../tools/build.ps1>) compiles the runtime first, checks the native payloads, and passes a fresh asset directory to the desktop [build script](<../apps/desktop/build.rs>). The script embeds exact bytes, sizes, and SHA-256 identities. Release builds statically link the Visual C++ runtime and check native imports before publication. Build and Publish copy only the launcher EXE. Recognized old sidecar files are removed after identity checks; unrelated files stay untouched.

## What happens on the target

- The chooser reads the embedded catalog without extracting files.
- Setup verifies both embedded boot payloads, then uses a private boot-recovery cache as the vendor's source directory. EFI installation and the journal still follow the vendor workflow.
- Game launch verifies and stages its runtime in a fresh temporary directory. The session owns that directory until the child stops. Normal exit removes it.
- Restore uses the boot journal. After successful vendor recovery, the app removes only cache files whose names, hashes, ownership marker, and journal identities match.

Saved settings and boot recovery data remain in AppData. Installed boot files remain on the EFI partition until Restore. The folder containing the EXE stays free of runtime, DLL, EFI, and catalog sidecars.

The cache preserves source bytes because the vendor needs them to prove ownership of an interrupted EFI copy. Runtime staging has no persistent cache. Cleanup rejects reparse points and changed ownership instead of following them into another directory.

## Checks

```powershell
.\external-everything.exe --inspect-bundle
.\external-everything.exe --verify-bundle
.\external-everything.exe --verify-runtime-launch
.\external-everything.exe --runtime --self-test
.\external-everything.exe --runtime --snapshot
```

`--inspect-bundle` verifies the four embedded identities without extraction. `--verify-bundle` checks temporary boot staging, the staged runtime contract, and cleanup; it leaves firmware unchanged. The runtime diagnostics use the same owned extraction path as game launch.

The [clean-folder test](<../tools/test-single-file.ps1>) copies only the EXE into a fresh directory, isolates Temp/AppData, and verifies embedded boot sources, attachment, native rendering, game handoff, and empty temporary staging after exit. The [GUI test](<../tools/test-single-file-ui.ps1>) opens PointBlank through the real chooser and closes the launcher, checking child ownership and cleanup.

The [bundle](<../apps/desktop/src/bundle.rs>) and [recovery](<../apps/desktop/src/recovery.rs>) fixtures cover malformed images, substituted hashes/catalogs, concurrent stages, reparse points, changed files, recovery retention, and journal-bound cleanup. Development Check/Test/Clippy builds deliberately exclude embedded release assets; use the build helper for the distributable.
