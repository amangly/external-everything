# PointBlank integration evidence

## Supported images

The [build profiles](<../crates/external-runtime/src/games/pointblank/offsets.rs>) preserve the supplied legacy image and register the independently checked installed image. The [engine](<../crates/external-runtime/src/engine.rs>) selects by full executable SHA-256, then checks the loaded image size and typed object layout.

| Profile | SHA-256 | SizeOfImage |
| --- | --- | --- |
| `pb_id_20261115` | `edeeeef3e76caa41b13c0a66762da95b90e989f09b8dff7d8480a9bd302ee084` | `0x01F6F000` |
| `pb_id_20261116` | `d7de2e99c257164ce8022ecf81dd1ea5fe1fa7e9c6ae8bba58bb78c4cb459b46` | `0x01F7C000` |

The November 16 executable is x86 PE32, product version `127.1.2611.16`. Its loaded RTTI supplied these exact primary vtables:

| Class | RVA |
| --- | --- |
| `CGameFramework` | `0x00E68644` |
| `i3Camera` | `0x00E2948C` |
| `i3ViewMatrixAttr` | `0x00E2F76C` |
| `i3ProjectMatrixAttr` | `0x00E2F6DC` |
| `Avatar` | `0x00EA4410` |
| `CGameCharaManager` | `0x00E734D4` |
| `i3MatrixObject` | `0x00E240DC` |

These values were recovered through TypeDescriptor → complete-object locator → vtable chains and checked against the historical class identities. They were derived separately rather than applying section-placement deltas to old constants.

## Observed live reads — November 17, 2026

Direct hyper-reV reads verified:

- Framework global RVA `0x01245088`, with aliases `0x0124C980` and `0x0124EC10`, resolves to the typed framework.
- Framework `+0xEC` resolves to the camera; camera `+0xD4` / `+0xD0` resolve to typed view/projection attributes. Matrices are stored at attribute `+0x30`; the observed viewport is **2560×1080**.
- Current-Avatar slot RVA `0x0126A100` resolves to the typed Avatar when populated. Health follows Avatar `+0xC38`, with ID at health `+0x30` and float value at `+0x34`. The observed sample held ID `0` and health `64`.
- Avatar `+0xB88` resolves to a typed matrix object; translation at `+0x50` yielded finite world coordinates.
- Bounded manager discovery subsequently produced **12 typed collection entries, 12 entities, zero rejected**. Live values included health `100`, `142`, `120`, `132`, and `105`; float values above 100 are retained.
- A projected estimated rectangle was `[2226.24, 555.00, 68.44, 190.11]` in the source viewport.

Machine-local records: [image identity and section coverage](<../build/pointblank-live-prefault-b92e786000b848bbb91947a31c2fc7fe/evidence.json>), [seven RTTI chains](<../build/pointblank-live-prefault-b92e786000b848bbb91947a31c2fc7fe/core-rtti.json>), [camera/Avatar chain and consistency rereads](<../build/pointblank-live-prefault-b92e786000b848bbb91947a31c2fc7fe/chain-proof-c14a95ee91374233b588c6881b9ac328.json>), and [full-collection frame](<../build/pointblank-collection-6e9ed0a515194a21a1002c2589b9f32d.json>). Generated captures remain outside Git; the profiles and validation code are versioned.

## Direct manager startup

The current profile also registers value-wrapper RVAs `0x012670B8` and `0x012670CC`. Their payload pointers are at `+0x0C`; adding the two payload words with x86 wrapping arithmetic resolves the manager. Two process lifetimes, native initialization/accessor code, and callback copy behavior support this path. The reader requires initialized zero-flag wrappers, stable wrapper/payload rereads, and the full typed collection proof. See [direct startup evidence and timings](<pointblank-direct-startup.md>). The legacy profile retains bounded discovery.

## Transport and launch checks

The [transport](<../crates/hyperrev-transport/src/lib.rs>) uses this fork's full-width CPUID ABI, including the actual target CR3. Native tests exercised read/write/restore against self-owned 32-byte and 65-KiB buffers and validated automatic current-process discovery. The PointBlank capture path reads game memory; these write tests affect only their own allocations.

Bulk testing exposed demand-zero local destination pages at a page crossing. The transport now CPU-touches each local buffer page before the hypercall. This materializes local pages; it is distinct from pinning, and a later paging failure remains an explicit read error.

The [launcher](<../apps/desktop/src/backend.rs>) verifies the packaged executable hash, runtime version/backend contract, started event, actual renderer-ready event, and owned-child cleanup. The [native verification script](<../tools/test-pointblank.ps1>) repeats transport, Direct2D/Direct3D11 rendering, and launcher checks.

## Rendering boundary

GPUI belongs to the loader only. The game [presentation layer](<../crates/external-runtime/src/presentation/mod.rs>) reuses Palepale's actual [Direct2D/DirectWrite ESP overlay](<../crates/external-runtime/src/common/overlay.rs>), [Direct3D11/ImGui menu renderer](<../crates/external-runtime/src/common/imgui_ui.rs>), and [shared menu chrome](<../crates/external-runtime/src/common/chrome.rs>). The game reader feeds those surfaces through a background worker, keeping memory discovery outside the render loop. Rendering/menu code follows the pinned reference rather than a second GPUI game pane or GDI box renderer.

The copied native stack passed 164 regression tests, Clippy, and runtime launch/cleanup checks. An initial live-render checkpoint painted 139 ESP frames with up to three projected entities; the [final DPI-aware report](<../build/renderer-verification-8a1e725916ec4e2baa9afa064de8c08c.json>) separately records 478 successful menu/overlay frames while the reader was waiting. Native menu close also exited cleanly with owned workers joined.

Full collection, single observed Avatar, and waiting states remain distinct. The boxes use the source's 1.8-unit standing-height presentation estimate. Bone locations, team fields, semantic foot/head anchoring, and pixel-exact body extents remain separate work; these observations establish the implemented camera/health/world-position/collection path.
