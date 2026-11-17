# Architecture

## Ownership

External Everything owns its Windows x64 GPUI launcher, route state, game chooser, attachment-check isolation, catalog, and external-process supervision. The pinned hyper-reV fork owns boot installation and its native attachment. The application calls the vendor backend; it has replaced the earlier shared-UI wrapper.

```text
Application GPUI loader
  -> app backend -> hyper-reV Setup / Restore -> firmware / ESP + recovery journal
  -> isolated same-executable probe -> hyperrev-transport -> attached hyper-reV
  -> attachment-ready Games chooser -> catalog + executable verification
  -> owned external child process -> child exit -> chooser
```

The [workspace](<../Cargo.toml>) contains the [desktop application](<../apps/desktop/Cargo.toml>) and [transport crate](<../crates/hyperrev-transport/Cargo.toml>). The [entry point](<../apps/desktop/src/main.rs>) handles the internal probe before normal UAC elevation and starts the [application UI](<../apps/desktop/src/ui.rs>).

## Current lifecycle

| Observation | Route |
| --- | --- |
| Probe pending | Checking |
| Attachment absent, installation absent | Setup |
| Attachment absent, recorded installation present | Restart required |
| Exact attachment test response and usable guest CR3 | Games chooser |
| Inspection/setup/probe failure | Error with details and retry |

Only attachment readiness opens the chooser. Setup success, installed firmware entries, and catalog contents independently grant zero game-launch capability. Setup and Restore call `hyperrev_loader::app::perform` through the [backend](<../apps/desktop/src/backend.rs>); they retain the vendor's validation, journaling, ownership checks, and manual-restart policy. A running attachment can remain responsive until the next Windows boot even after Restore.

The [UI](<../apps/desktop/src/ui.rs>) opens a real chooser window before removing the previous loader window. Work runs through background tasks, and a failed window handoff leaves the existing window alive. The chooser stays alive during an external session; completion refocuses it, asynchronously rechecks readiness/catalog state, and returns to the loader if the attachment is absent. Child-exit errors survive that recheck in the details. Every later launch also performs fresh attachment checks. The [runtime guide](<../games/README.md>) specifies the current child integration boundary.

### Isolated readiness check

The [probe controller](<../apps/desktop/src/probe.rs>) starts the same executable with `--internal-attachment-probe`, captures a bounded JSON result, and enforces a three-second deadline. A crash, timeout, invalid response, or unusable CR3 surfaces as an error rather than opening the chooser. The parent reaps a failed probe.

The schema-1 result contains `response` and `cr3`. [Transport classification](<../crates/hyperrev-transport/src/lib.rs>) requires the fork's exact test response, then a nonempty CR3 physical-frame field. It follows the pinned vendor calling convention. This is observed protocol readiness, rather than cryptographic attachment identity or a claim about a game backend. The transport currently exposes readiness only; guest-memory read/write APIs remain future work.

### Catalog and external runtime

The executable-relative [catalog](<../games/catalog.json>) begins with an empty `games` array. Named entries without an executable/hash pair appear pending. [Catalog loading](<../apps/desktop/src/catalog.rs>) enforces schema, identifier uniqueness, bounded sizes, relative executable paths within the games directory, and link rejection. Before spawning, it verifies the catalog SHA-256 and x64 PE image shape.

[Runtime launch](<../apps/desktop/src/backend.rs>) checks readiness, resolves and verifies the catalog executable, checks readiness again, and starts:

```text
<verified-runtime.exe> --game <game-id>
```

The child owns its external window and game logic. `RuntimeSession` owns the process, exposes `try_wait`, and stops/reaps a still-running child when dropped. The UI polls completion and restores chooser focus. A nonzero child exit or monitoring error is surfaced. Successful spawn establishes process creation only: game-window readiness acknowledgements and bidirectional runtime IPC remain later work. The game argument follows Palepale's `--game` spelling; its binary compatibility checks and event protocol remain separate integration work.

## Vendor boundary

The [submodule declaration](<../.gitmodules>) identifies [amangly/hyper-reV](<https://github.com/amangly/hyper-reV>); the parent gitlink pins the actual revision. Shared boot/attachment changes belong in that fork, followed by explicit review and a pin update. The application does not fork a second Setup/Restore implementation.

Boot entries remain **hyper-reV** and **Safe Windows**. The recovery journal remains `%LOCALAPPDATA%\hyper-reV\boot-install.json`. The [vendor loader guide](<../vendor/hyper-reV/loader/README.md>) defines prerequisites, repeated setup, journal-based Restore, and stock Windows verification. Vendor debugger and CLI tools remain separate from this app bundle.

## Packaging

The [build helper](<../tools/build.ps1>) builds/checks the root workspace and packages prebuilt matching boot payloads. Its default native source is the vendor Release directory; `-PayloadDirectory` selects another matching build. Build stages the following under `dist/`; `-Publish` also copies it to `%LOCALAPPDATA%\External Everything\bin`:

```text
external-everything.exe
uefi-boot-standalone.efi
hyperv-attachment.dll
games/catalog.json
```

The helper copies the catalog, rather than deploying future game binaries. Both boot payloads sit beside the application for vendor discovery. Native payload compilation remains a separate vendor build; generated artifacts stay outside Git. The [vendor sync helper](<../tools/sync-vendor.ps1>) explicitly fetches and detaches a candidate revision for review rather than silently tracking a branch.

## Reference and deferred scope

The [pinned Palepale flow comparison](<reference-flow.md>) records source provenance and the intentional changes: strict attachment-before-selection, application-owned GPUI windows, retained process ownership, and manual restart. License/authentication, remote asset delivery, automatic self-update, and hardware spoofing are intentionally excluded. The [reserved external-runtime source home](<../crates/external-runtime/README.md>) corresponds to Palepale's `df` crate. Concrete game backends, game-process attachment, memory APIs, and runtime readiness IPC remain later work; the empty catalog reflects that boundary.

Keep app lifecycle/game code here, shared boot/ABI code in the maintained fork, and GPL-3.0-only licensing and upstream attribution intact. Review compatibility and run application checks when accepting a vendor update.
