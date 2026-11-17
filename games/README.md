# Game runtimes

The application owns the GPUI loader and chooser. Games appear only after verified attachment readiness. The external child uses Palepale's copied Direct2D/DirectWrite ESP and D3D11/ImGui/Chrome menu, with its own game reader; GPUI stays launcher-only.

## PointBlank packaging

The source [catalog](<catalog.json>) registers `pointblank` by ID/name only. The [build helper](<../tools/build.ps1>) builds the [runtime crate](<../crates/external-runtime/README.md>), packages it beneath the application, and generates the published catalog entry:

```json
{
  "schema": 1,
  "games": [
    {
      "id": "pointblank",
      "name": "PointBlank",
      "executable": "pointblank/external-runtime.exe",
      "sha256": "[generated lowercase runtime SHA-256]"
    }
  ]
}
```

Build stages the runtime and generated catalog under `dist/`; `-Publish` also copies them to `%LOCALAPPDATA%\External Everything\bin`. The source registration stays free of machine-specific binary hashes. A name-only entry is pending when used without the generated published entry.

[Catalog validation](<../apps/desktop/src/catalog.rs>) rejects paths escaping the games directory, link/reparse traversal, duplicate IDs, incomplete executable/hash pairs, changed hashes, and invalid x64 PE shape. The runtime's SHA-256 identifies the tool binary; it is independent of the PointBlank game-image hash required by the reader.

## Launch and return

The [launcher backend](<../apps/desktop/src/backend.rs>) verifies attachment and the catalog executable, then delegates to [runtime startup](<../crates/external-runtime/src/launch.rs>):

```text
external-runtime.exe contract --json
external-runtime.exe --game pointblank
```

The contract checks schema, identity, exact version, backend ABI, and selected-game metadata. Started and renderer-ready named events have eight-second and twenty-second deadlines respectively. **Ready follows successful initialization/first frames of the Direct2D and D3D11/ImGui surfaces**, separately from profile admission and match state. Startup failure stops/reaps the child. END or the shared menu's quit action returns to the still-open chooser; attachment/catalog state is rechecked. Closing the launcher stops its owned runtime.

## PointBlank profiles and limits

The [profile-aware reader](<../crates/external-runtime/src/games/pointblank/offsets.rs#L46-L95>) supports two exact game-image identities:

| Profile | SHA-256 | PE32 image size |
| --- | --- | --- |
| `pb_id_20261115` | `edeeeef3e76caa41b13c0a66762da95b90e989f09b8dff7d8480a9bd302ee084` | `0x01f6f000` |
| `pb_id_20261116` | `d7de2e99c257164ce8022ecf81dd1ea5fe1fa7e9c6ae8bba58bb78c4cb459b46` | `0x01f7c000` |

The current November 16 image, product `127.1.2611.16`, has fresh RTTI, camera/Avatar field proof, and live collection verification. Only those full hashes and matching image sizes select profiles; unsupported builds stay rejected. The [evidence summary](<../crates/external-runtime/README.md#verified-profiles-and-remaining-limits>) separates verified data capture from unresolved presentation semantics.

A bounded current-profile scan verified a live manager/vector and [FullCollection with 12 typed entries, 12 entities, zero rejected](<../build/pointblank-collection-6e9ed0a515194a21a1002c2589b9f32d.json>), preserving float health above 100 and producing projected rectangles. `SingleAvatar` remains the distinct observed-slot fallback. Bones, teams, semantic feet, and exact box alignment remain open; standing box height remains a 1.8-world-unit presentation estimate.

The reused Chrome shell provides PointBlank ESP/MISC/CFG tabs and persistent settings; INSERT toggles it and END exits. Unsupported aim/bones are absent. The copied renderer's own native eight-second checkpoint passed: 480 overlay frames, 206 menu frames, 139 live ESP frames, 14 latest entities, peak three projected entities, and both ready flags. The final DPI-aware [report](<../build/renderer-verification-8a1e725916ec4e2baa9afa064de8c08c.json>) separately passed 478 overlay/menu frames and both ready flags while waiting (zero entities/ESP); it never inherits the earlier live counts. Final started/ready/child-cleanup, native transport, and loader-caption checks also passed. The self-test touches owned 32-byte and 65-KiB buffers rather than game state. See [runtime details](<../crates/external-runtime/README.md>) for provenance and diagnostics.

Additional game backends remain later work. Keep their sources in this repository, shared attachment ABI changes in the maintained fork, and transport calls behind the [transport crate](<../crates/hyperrev-transport/src/lib.rs>).
