# DD-up for HarmonyOS

A remote client for herdr-core on your Mac, through `ddup-bridge` (D27).

- Setup and daily use: [`docs/harmony/SETUP.md`](../../docs/harmony/SETUP.md)
- Design and wire protocol: [`docs/harmony/DESIGN.md`](../../docs/harmony/DESIGN.md)
- Device acceptance: [`tests/gui/HARMONY_CHECKLIST.md`](../../tests/gui/HARMONY_CHECKLIST.md)

| Path | What |
|---|---|
| `entry/src/main/ets/core/` | Protocol and state, no UI: `Crypto` (DDB1), `Secure` (encrypted WebSocket), `Hc` (herdr-core framed protocol), `Agents` (grouping per labels.md), `Session`, `Vault` (Asset Store) |
| `entry/src/main/ets/pages/` | `Index` (pairing, agent list), `Terminal` (live screen, quick keys, send) |
| `entry/src/main/resources/rawfile/term/` | xterm.js 6.0.0 (MIT, `LICENSE-xterm.txt`; npm sha512 checked) and the watch-only page |
| `entry/src/test/ddb1-vectors.json` | DDB1 test vectors, the same as `bridge/testdata/vectors.json` |

`core/` runs unchanged under Node with shimmed kits against the bridge and the test
core: `tests/integration/harmony-logic` (or `scripts/test --bridge`). That checks the
logic, not ArkTS compilation or the UI; those need DevEco Studio and a device.
