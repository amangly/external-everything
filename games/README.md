# Game packages

External Everything's goal is external cheats for all games using hyper-reV's CPUID-based hypercalls. Each game gets its own module and [catalog](<catalog.json>) entry. PointBlank is the first implemented game.

The [build helper](<../tools/build.ps1>) embeds the generated catalog and runtime inside `external-everything.exe`. The generated catalog binds each packaged runtime to its SHA-256. The launcher checks that identity, the runtime contract, and attachment readiness before launch.

The runtime runs from an owned temporary directory. Closing the game session stops the child and removes that directory. The distribution folder keeps one EXE. See [single-file packaging](<../docs/single-file.md>).

The tool's binary hash and the accepted PointBlank game hash serve separate checks. Game profiles live in the [reader](<../crates/external-runtime/src/games/pointblank/README.md>).

Choose **PointBlank** in the launcher. Press **INSERT** for the menu and **END** to exit. The chooser stays open until the runtime returns. See the [runtime guide](<../crates/external-runtime/README.md>) for controls and settings.
