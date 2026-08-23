# windKrill

Fully open-source, pixel-faithful reimplementation of [WindTerm](https://github.com/kingToolbox/WindTerm)'s
functionality, fixing its known critical defects (crash-on-Ctrl+C, session-dialog freeze,
ZModem filename mojibake, SFTP disconnects). **Windows-first**, other platforms later.

## Stack (方案 A)

| Layer | Choice | Why |
|---|---|---|
| Shell / UI | Tauri 2 + TypeScript + Vite | CSS-grade pixel fidelity, hot reload = fast AI iteration loop |
| Terminal engine | Rust workspace (`src-tauri/crates/`) | Compiler as AI reviewer; memory safety by construction |
| VT parsing | `vte` crate behind `krill-vt` facade | vttest-grade correctness without hand-rolling state machines |
| Screen model | `krill-core` (own) | folding / timestamps / command blocks / compressed scrollback are our differentiators |
| Transport | `krill-transport` (ConPTY first, russh later) | trait-based so SSH/Telnet/Serial slot in behind one interface |

Reused Apache-2.0 components from WindTerm's open-sourced `src/` are tracked in
[docs/specs/windterm-reuse.md](docs/specs/windterm-reuse.md).

## Layout

```
src/                  TS frontend (Vite)
src-tauri/
  Cargo.toml          Rust workspace
  crates/
    krill-vt          VT/xterm parser facade
    krill-core        screen buffer: grid, blocks, folding, scrollback
    krill-transport   ConPTY / SSH / Telnet / Serial transports
    krill-app         Tauri command layer
docs/specs/           WindTerm behavior specs driving "pixel-faithful" acceptance
.github/workflows/    CI: fmt, clippy, test, tsc
```

## Milestones

- **M0** ✅ repo skeleton, CI, parser→screen pipeline with tests
- **M1** VT compatibility (vttest regression in CI), 256c/truecolor, mouse protocol
- **M2** ConPTY + local shells (PowerShell/cmd/WSL), desktop window shell
- **M3** GUI skeleton: docking, tabs, splits, theme engine
- **M4** SSH (russh) + sessions + auto-login; SFTP with resume; ZModem w/ encoding fix
- **M5** advanced feature harvest: folding → timestamps → sync input → palette → tmux integration

## Development

```bash
npm install            # frontend deps
npm run dev            # Vite dev server (:1420)
cargo check --workspace        # from src-tauri/
cargo test -p krill-vt -p krill-core
```

Note: building the full desktop app (`krill-app`) requires the platform webview deps;
on Linux CI/dev boxes use the workspace `default-members` subset.

## License

Apache-2.0.
