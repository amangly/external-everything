# External runtime

Shared runtime for External Everything's external game cheats. Memory reads use hyper-reV's CPUID-based hypercalls. PointBlank is the first implemented game; more games will use their own readers and features.

ESP uses Direct2D/DirectWrite. Menus use Direct3D11/ImGui and shared controls. GPUI stays in the launcher.

## Controls

- **INSERT**: show or hide the menu.
- **END** or **Quit overlay**: exit and return to the chooser.
- **ESP**: master switch, boxes, health labels, and color.
- **MISC**: crosshair and runtime details.
- **CFG**: save or load three settings slots.

<a id="verified-profiles-and-remaining-limits"></a>

Settings live under `%LOCALAPPDATA%\External Everything\pointblank`. The runtime checks the PointBlank executable against its [build profiles](<src/games/pointblank/offsets.rs>). Team filtering, skeletons, and exact body bounds are unfinished.

## Launch

The launcher verifies the runtime hash and contract, starts the child, and waits for both renderers to paint. It keeps the chooser open and stops the child when the launcher closes.

```powershell
.\external-runtime.exe --game pointblank
.\external-runtime.exe --self-test
.\external-runtime.exe --snapshot
.\external-runtime.exe --render-smoke 5
```

The self-test reads, writes, and restores its own buffers. The render smoke runs for at most ten seconds and prints frame counts.

[Engine](<src/engine.rs>) handles process discovery and frame reads. [PointBlank](<src/games/pointblank/README.md>) validates game objects and projects boxes. [Presentation](<src/presentation/mod.rs>) connects those frames to the native [renderers](<src/common/mod.rs>).
