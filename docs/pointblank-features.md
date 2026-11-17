# PointBlank feature offsets

The external runtime now includes native name tags, the recovered client room-kick toggle, Scope 3Q / 3Q macros, dot/cross crosshairs, and five menu themes. Body wallhack remains under verification.

The game-memory sites below apply to `pb_id_20261116`, executable SHA-256 `d7de2e99c257164ce8022ecf81dd1ea5fe1fa7e9c6ae8bba58bb78c4cb459b46`, image size `0x01F7C000`. Addresses are module-relative RVAs. The legacy profile keeps its existing reader and excludes these patches.

## Code patches

| Feature | Guard RVA | Patch RVA | Original → enabled |
| --- | --- | --- | --- |
| Native name tags | `0x0067922C` | `0x00679237` | `00 → 01` |
| Client room-kick handler | `0x0073D4D6` | `0x0073D4E1` | `78 → EB` |

Name-tag guard, 21 bytes:

```text
6A 12 81 C1 3D 06 00 00 58 C6 01 00 8D 49 08 83 E8 01 75 F5 C3
```

The changed byte is the immediate in `mov byte ptr [ecx], 0`. The function resets 18 nickname-target flags at context `+0x63D`, stride eight. Callers and consumers link `CGameCharaCollisionContext::UpdateNickOnTarget` to `CGameCharaHUDContext::ShowCharacterNick`.

Kick-handler guard, 39 bytes:

```text
55 8B EC 8B 45 08 56 8B F1 85 C0 78 15 0F 9F C0 0F B6 C0 50
E8 3A D9 FF FF 6A 08 8B CE E8 16 DF DA FF 5E 5D C2 08 00
```

Changing `js +0x15` to `jmp +0x15` skips this event `0x801` callback's body and keeps its epilogue. The callback reaches the room-kicked messages and state transition. This changes the client handler, not server membership. The supplied AntiKick signature starts with `48`, while its patch assumes `78`; its initializer is also uncalled. The port uses the recovered branch and complete function guard.

Both full guards are unique in the captured text. Eighteen fresh hypercall reads matched both earlier process captures. The [offset ledger](<../build/pbzptid-offsets/offset-ledger.json>) records code references, named parser paths, and original/enabled windows.

The [controller](<../crates/external-runtime/src/games/pointblank/patches.rs>) checks the exact profile and loaded PE, validates the complete guard, owns each byte before writing, verifies readback, and rolls back failed transactions. It restores only its own bytes and preserves changed guards. Normal runtime exit restores the patches; launcher shutdown first requests a normal child close. Both features default off.

The [native verification](<../build/feature-verification-af9bf5af09b1407a98787472515ec8eb/evidence.json>) tested separate and combined writes, checked both enabled guards while rendering, then checked both original guards after closing the menu. Gameplay effects and server behavior are separate from this byte/restore test.

## Input and display features

Scope 3Q uses a right-button press, then delay → left click → delay → `3` → delay → `Q`. The 3Q mode uses a left-button press, then delay → `3` → delay → `Q`. Keys `3` and `Q` are virtual keys `0x33` and `0x51`. Synthetic holds last at least 20 ms. Delays are 25, 50, 75, 100, or custom 0–9999 ms.

The [owned macro worker](<../crates/external-runtime/src/presentation/macros.rs>) checks the selected foreground PID, triggers once per press, cancels on menu/focus/config changes, and releases its synthetic input on exit. Thirteen deterministic fixtures test the sequences and cancellation; native input stays excluded from test builds.

Dot crosshair is a centered red 4×4 square. Cross is green, with 20×2 and 2×20 segments. The previous segmented style remains available. [Settings](<../crates/external-runtime/src/presentation/settings.rs>) also save Mint, Cyan, Crimson, Purple, or Gold theme selection.

## Body wallhack research

The supplied body feature hooks a D3D9 draw and temporarily sets depth comparison to `ALWAYS`. Its constants `16/42/82` are COM vtable slots; strides `36/44/48/52` are GPU layout tests, rather than game memory offsets.

Static analysis recovered these candidate paths:

| Object | Field or RVA |
| --- | --- |
| Avatar-owned AttrSet | Avatar `+0x88` |
| Avatar scene root | wrapping sum of Avatar `+0x120` and `+0x124` |
| Node child vector | `+0x30/+0x34/+0x38` |
| AttrSet attribute vector | `+0x80/+0x84/+0x88` |
| AttrSet bypass flag | `+0x48 & 0x20000` |
| `i3DepthStencilAttr` vtable | RVA `0x00E35E34` |
| Depth enable / write-enable | attribute `+0x28/+0x29` |
| Depth comparison / dirty flags | attribute `+0x2C/+0x24` |
| Cached state pointer | attribute `+0x58` |

The renderer maps comparison `8` to `D3DCMP_ALWAYS`. Attribute push/pop scopes state to a subtree, but construction initially sets the AttrSet bypass bit. A live Avatar-owned depth descriptor, sharing rules, child overrides, and restoration still need verification. The latest bounded probe found an empty player collection, so these fields remain research data and the menu excludes a body toggle. See the [body ledger](<../build/pbzptid-body/body-offset-ledger-v3.json>).

Current regression checkpoint: 238 passed, four environment-specific tests ignored; Clippy and Release build passed. The native patch/restore check and launcher/render checks also passed. Generated evidence stays outside Git.
