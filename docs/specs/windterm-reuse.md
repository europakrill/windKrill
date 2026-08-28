# Reusable Apache-2.0 components from WindTerm

WindTerm's `src/` directory is Apache-2.0. These components can be vendored
(ported to Rust or bound) with attribution; each entry needs a review task
before adoption.

**Current status:** no source file in windKrill is a direct port of a WindTerm
source file. The table below records possible future reference points, not code
already incorporated into the repository. Any future derived implementation
must update this document, `NOTICE`, and the affected source-file headers.

| WindTerm component | What it gives us | Adoption plan |
|---|---|---|
| `src/Pty` (ptyqt rewrite) | ConPTY/Unix PTY handling battle-tested by WindTerm | Port semantics into `krill-transport::local` (M2); do not bind C++ directly |
| `src/Protocols/TelnetProtocol.*` | Full Telnet implementation | Port to Rust in `krill-transport` (M4+) |
| `src/Utility/CircularBuffer.h` | Fast ring buffer template | Reference design for scrollback ring in `krill-core` |
| `src/Utility/Cryptographic.*` | PBKDF2-based secret protection | Reference only — we use Windows Credential Manager / DPAPI + PBKDF2 via system BCrypt, never custom crypto |
| `src/Onigmo` | Regex engine matching non-adjacent memory blocks | Only if our search layer needs gap-buffer regex; otherwise `regex` crate suffices |
| `src/libssh` fork | Pageant support, extra HMACs, external socket | Feature checklist for russh config (M4): pageant-style agent, etm HMACs |

## Attribution requirement

When a ported file derives from WindTerm source, keep the original
Apache-2.0 header and add:

```
Portions Copyright (c) kingToolbox / WindTerm contributors,
licensed under the Apache License, Version 2.0.
Ported to Rust as part of windKrill.
```

## Hard boundaries (do NOT reuse)

- Any binary assets: theme images, icons, screenshots from the repo/images
  or the distributed app — those are not part of the open-sourced code and
  must not be copied.
- The closed-source functionality itself is re-implemented from behavior
  specs (`docs/specs/`), never decompiled.
