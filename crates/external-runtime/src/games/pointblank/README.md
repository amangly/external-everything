# PointBlank reader

Reads camera, player, health, and world-position data from the x86 `PointBlank.exe` process through hyper-reV. The runtime uses it to draw boxes and health labels.

## Builds

[Profiles](<offsets.rs>) pin the full executable SHA-256, image size, and module offsets:

| Profile | Product version | Image size |
| --- | --- | --- |
| `pb_id_20261115` | `127.1.2610.24` | `0x01F6F000` |
| `pb_id_20261116` | `127.1.2611.16` | `0x01F7C000` |

The engine checks the file hash before capture. The reader checks the loaded PE image, object types, pointer ranges, and collection bounds. New builds need a checked profile.

## Frames

`FullCollection` contains the validated player vector. `SingleAvatar` contains one observed callback slot while collection discovery is unresolved. Waiting or changed-data states clear rejected data.

Each entity has an address, float health, world position, and optional screen rectangle. Off-screen entities keep their world data. Boxes use a 1.8-unit height estimate; bones, team filtering, and exact foot/head anchors are unfinished.

[Reader code](<reader.rs>) handles capture and discovery. [Geometry](<geometry.rs>) projects positions. [Fixtures](<tests.rs>) test types, bounds, matrices, and collection changes.

The original offsets came from the supplied `pointblank-cheat` sources. The newer profile adds checked RTTI and live field reads. PBZPTID's NameTag/AntiKick patch patterns are unused. See the [profile evidence](<../../../../../docs/pointblank-profile.md>).
