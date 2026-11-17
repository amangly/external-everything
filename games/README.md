# Game runtimes

The application owns the loader and game chooser. Game-specific external runtimes live in this directory and appear in the chooser only after the attachment check succeeds.

The initial [catalog](<catalog.json>) is empty: game backends are still to be integrated. Add a reviewed runtime with its x64 executable path and SHA-256:

```json
{
  "schema": 1,
  "games": [
    {
      "id": "[game-id]",
      "name": "[Game name]",
      "executable": "[game-id]/external.exe",
      "sha256": "[lowercase SHA-256]"
    }
  ]
}
```

Executable paths are relative to this games directory. An entry with just `id` and `name` appears as pending rather than pretending its backend is installed. Paths outside this directory, duplicate IDs, missing hashes, and changed binaries are rejected.

The launcher starts a verified runtime with `--game [game-id]`. Each game runtime owns its external window and game logic. Closing that runtime returns to the chooser; the launcher rechecks attachment readiness before opening another. Closing the launcher stops its owned child runtime.

Keep backend implementation sources in the application repository. Shared attachment ABI changes belong in the hyper-reV fork; use the [transport crate](<../crates/hyperrev-transport/src/lib.rs>) for the pinned protocol boundary rather than duplicating register constants in games.
