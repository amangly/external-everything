# External runtime — next port

This is the reserved source home for the per-game external application, corresponding to [Palepale's df crate](<https://github.com/amangly/palepale/tree/main/crates/df>). Runtime implementation is pending; this directory is deliberately outside the Cargo workspace until that port builds.

Keep the reference's useful split:

```text
external-runtime/
  src/main.rs          --game dispatch and runtime contract
  src/common/          window, rendering, input, configuration, startup signals
  src/games/<game>/    game process, reader, offsets, feature state, external UI
```

Game-independent attachment calls belong in the [transport crate](<../hyperrev-transport/src/lib.rs>). Boot installation stays in the [hyper-reV vendor](<../../vendor/hyper-reV/loader/README.md>). Application routing stays in the [desktop launcher](<../../apps/desktop/src/ui.rs>).

The launcher already starts a catalog-verified executable with `--game <id>`, monitors its process, and rechecks attachment readiness when it exits. The [game catalog guide](<../../games/README.md>) defines packaging. Palepale's runtime contract probe, started/ready acknowledgements, and concrete game modules still need integration; successful process creation alone is distinct from game readiness.
