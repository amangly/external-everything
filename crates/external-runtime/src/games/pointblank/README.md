# PointBlank typed external reader

This is a source-backed Rust port for one pinned x86 image, using the host's direct hyper-reV virtual-memory reader. The module performs reads and geometry only. It owns neither process discovery nor a command bridge.

## Identity and provenance

- Process: `PointBlank.exe`.
- Remote image: PE32, machine `0x014c`, exact `SizeOfImage = 0x01f6f000`.
- Ledger SHA-256: `edeeeef3e76caa41b13c0a66762da95b90e989f09b8dff7d8480a9bd302ee084`.
- The host first checks the on-disk executable hash, before calling the typed reader. Every capture independently checks remote MZ/PE/machine/optional-header/image-size identity before following offsets. The remote PE check supplements the hash; it does not calculate a hash of mapped memory.

The following user-specified local references were actually read for this port:

| Reference | Evidence adopted |
| --- | --- |
| [Verified ledger](<C:/Users/Administrator/Downloads/pointblank-cheat/internal/offsets_verified.h#L5-L60>) | Build hash/size; module RVAs; typed camera chain; current-Avatar slot versus collection; float health; transform translation; manager implementation/vector fields. |
| [Ready Rust reader](<C:/Users/Administrator/Downloads/pointblank-cheat/rust_external/src/game.rs>) | Existing external typed-reader flow, offsets and row-vector matrix product; dynamic manager discovery validation. |
| [Rust ABI](<C:/Users/Administrator/Downloads/pointblank-cheat/rust_external/src/abi.rs>) | Existing entity/frame meanings, readiness states and explicitly empty skeleton data. The legacy shared-memory/IPC ABI itself is intentionally absent here. |
| [Stronger C++ frame builder](<C:/Users/Administrator/Downloads/pointblank-cheat/internal/frame_builder.h#L36-L348>) | Scaled pivoted nonsingularity, orthonormal view/perspective proof guards, frame-local bulk snapshots with field fallback, vector uniqueness/limits/backlink, consistency rechecks, matrix-object RTTI. |
| [Existing Rust overlay geometry](<C:/Users/Administrator/Downloads/pointblank-cheat/rust_external/src/overlay_main.rs#L197-L240>) and [C++ presentation](<C:/Users/Administrator/Downloads/pointblank-cheat/internal/payload_direct.cpp#L128-L155>) | Row/column indexing equivalence and viewport projection; estimated rectangle presentation. |
| [PBZPTID legacy header](<C:/Users/Administrator/Documents/PBZPTID/Dll/Offsets.h>) | Contains only NameTag/AntiKick byte-patch patterns; supplies zero typed runtime offsets to this implementation. |

The legacy ledger describes prior source observations across two process lifetimes. This port adds synthetic fixtures and a separately admitted current profile. Later integration verified live typed camera/Avatar chains and a 12-entity full collection through hyper-reV; see the [profile evidence](<../../../../../docs/pointblank-profile.md>).

## Host contract

```rust
pub trait Memory {
    fn read(&self, address: u64, out: &mut [u8]) -> anyhow::Result<()>;
}

pub fn capture(
    memory: &impl Memory,
    image_base: u32,
    manager: Option<u32>,
) -> anyhow::Result<Frame>;

pub fn find_manager(
    memory: &impl Memory,
    base: u32,
    regions: &[(u64, usize)],
    byte_budget: usize,
) -> anyhow::Result<Option<u32>>;
```

The host implements `Memory::read` with its selected target's direct read-virtual-memory operation. A successful call fills the complete slice. The reader issues contiguous object/vector snapshots; it never converts those reads into the old bridge's eight-byte commands.

`Frame` carries `status`, `width`, `height`, `entities` and `manager`. `Entity` carries the x86 object `address`, original float `health`, verified body-matrix `position`, and optional screen rectangle `[left, top, width, height]` in source-camera pixels. An entity behind the camera or outside the presentation bounds retains data with `screen = None`.

The row-major matrix product remains `view * projection` for row-vector points; this port projects it directly instead of applying the legacy shared ABI's transpose. Source camera validation includes scaled pivoted nonsingularity, rigid/orthonormal view rotation and the sparse perspective form.

## Collection discovery and frame states

- `FullCollection`: a supplied typed manager, matching implementation backlink, aligned ordered vector bounds, at most 128 unique typed Avatar pointers. Empty valid vectors are still full-collection states.
- `SingleAvatar`: `manager = None` and the profile's observed callback slot (legacy RVA `0x0125e100`, current RVA `0x0126a100`) yields exactly one typed Avatar. This slot is explicitly outside collection discovery.
- `WaitingCamera` / `WaitingManager`: required objects or proof guards are unavailable.
- `CollectionInvalid`: a supplied manager/vector failed validation. The observed slot is kept separate rather than used as an implicit replacement.
- `CollectionChanged` / `SingleAvatarChanged`: consistency rechecks detected mutation; all entities from that frame are discarded.

Collection bounds, implementation ownership, every vector entry and every entry's type are rechecked each frame. Entries are validated for uniqueness, rather than silently deduplicated. Camera references, types, dimensions and matrix snapshots are also reread; a moving snapshot can therefore produce a transient discarded frame. Avatar matrix/health references are checked again before accepting an entity. Arithmetic is checked in the x86 address range `[0x10000, 0x100000000)`; field additions never wrap.

Manager discovery still lacks a verified stable module owner. The host supplies committed/private/readable regions with guard/no-access mappings excluded. The scanner compares the relocated RTTI-backed manager vtable, then proves a nonempty stable typed collection. It reads slabs up to 64 KiB and limits **all attempted scan and proof bytes**, including failed attempts/fallbacks, to `byte_budget`; the two mandatory PE-identity reads are additional. Exhaustion yields `None`. The host should throttle discovery and cache only a candidate that remains valid under capture's per-frame checks.

## Deliberately unresolved semantics

- The 1.8-world-unit standing height and 0.36 width/height ratio are **presentation estimates**, rather than verified bones or body dimensions. Transform translation is verified storage; semantic foot anchoring and exact in-game box alignment remain open.
- Health is a float, with source captures including 132. The port retains finite values from 0 through 10000 without clamping to 100 or rounding.
- Team/local-player classification, visibility, names and live skeleton storage remain unverified and are absent from `Entity`. Physics-shape pointers are excluded rather than mislabeled as joints.
- New executable acceptance requires a new source-backed ledger/hash review, rather than automatically adapting existing offsets.

Pure memory fixtures construct the PE and typed framework/camera/attributes, collection/Avatar, health and transform chains. They cover relocation, row-vector W2S and rectangle dimensions, corrupted pointers/types/bounds, float health, 128-entry blocks, duplicates, padding fallbacks, singular/nonfinite camera data, mid-frame changes and strict discovery budgets. The root integrates and runs the crate's tests/build; this module's fixture coverage is separate from live-game validation.

## Explicit build profiles

`BuildProfile` is a public `Copy`/`Debug` value containing image identity and module RVAs. `capture_profile`, `validate_image_profile`, and `find_manager_profile` use the explicitly supplied profile; their original entry points and `IMAGE_SIZE`/`EXPECTED_SHA256` retain `LEGACY_PROFILE` (`pb_id_20261115`) exactly. Object-field offsets remain the verified shared values, with all existing per-frame type and consistency guards retained. The host owns profile selection and checks the full on-disk SHA-256 plus exact image size; the reader performs neither automatic selection nor mapped-image hashing. Synthetic relocation tests use arbitrary globals/vtables beyond the legacy image and carry zero game-build support meaning.

| Profile | Identity and evidence |
| --- | --- |
| `CURRENT_PROFILE` (`pb_id_20261116`) | SHA-256 `d7de2e99c257164ce8022ecf81dd1ea5fe1fa7e9c6ae8bba58bb78c4cb459b46`, `SizeOfImage = 0x01f7c000`. Root's [captured RTTI proof](<../../../../../build/pointblank-live-prefault-b92e786000b848bbb91947a31c2fc7fe/core-rtti.json>) supplies the seven independent class identities. Root's [direct-read field-chain evidence](<../../../../../build/pointblank-live-prefault-b92e786000b848bbb91947a31c2fc7fe/chain-proof-c14a95ee91374233b588c6881b9ac328.json>) records the typed framework/camera/view/projection chain, stable 2560×1080 matrices and the typed current Avatar's health/body-matrix fields in PID 10432. A subsequent [live full-collection capture](<../../../../../build/pointblank-collection-6e9ed0a515194a21a1002c2589b9f32d.json>) verified 12 typed entries and entities with zero rejected. Bones, team classification and semantic foot anchoring remain separate; manager discovery still uses a bounded typed scan rather than a stable module owner. |
