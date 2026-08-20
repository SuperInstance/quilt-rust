# Cortex Agenda

> The first file the waking cortex reads — `postStartCommand` prints it and
> the devcontainer opens it. The **last commit of every burst rewrites this
> file**: the cortex dreams by rewriting tomorrow's agenda.
> Full contract: [docs/codespace-cortex.md](../docs/codespace-cortex.md).

## Standing order

```
wake -> read this file -> git pull -> think in steps of <= 25 min
     -> each step ends in a commit -> rewrite this file -> sleep
```

## Now (highest priority first — one think-step each, ~25 min)

- [ ] Fix one pre-existing formula/program test failure
      (`cargo test -p quilt-core`; see README "Honest disclosure")
- [ ] Wire `fire_listener` into the propagation loop
      (validated + traced but never fired — `packages/core/src/cells/listener.rs`)
- [ ] Wire `CellLedger` into `QuiltEngine` evaluation paths
      (the proprioceptive record — `docs/cell-ledger.md` §9)

## Parked (context, not commitment)

- [ ] Relay poll: fetch limb edges from lucineer-relay on wake
      (cortex polls, never listens — `docs/fleet-as-fractal-jepa.md` §1)
- [ ] MCP handoff gap: `quilt serve` loads the sheet then hands an empty
      server to `serve_stdio` (README FAQ, `packages/mcp/src/lib.rs`)

## Dream log (one line per wake — newest last)

- (none yet — the first wake appends here)
