# Game packages

External Everything's goal is external cheats for all games. Each game gets its own module and [catalog](<catalog.json>) entry. PointBlank is the first implemented game.

The [build helper](<../tools/build.ps1>) packages the runtime and generates its executable path and SHA-256:

```text
games/
  catalog.json
  pointblank/
    external-runtime.exe
```

The source catalog keeps the game ID and name. The published catalog adds the binary path and hash. The launcher checks that hash, the runtime contract, and hyper-reV readiness before opening PointBlank.

The tool's binary hash and the accepted PointBlank game hash serve separate checks. Game profiles live in the [reader](<../crates/external-runtime/src/games/pointblank/README.md>).

Choose **PointBlank** in the launcher. Press **INSERT** for the menu and **END** to exit. The chooser stays open until the runtime returns. See the [runtime guide](<../crates/external-runtime/README.md>) for controls and settings.
