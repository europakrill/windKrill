# Defects we are committed to fixing (acceptance criteria attached)

Sourced from WindTerm's issue tracker; each becomes a regression test.

## D1 Crash on Ctrl+C terminating `tail`-style scripts
- **WindTerm ref**: "[毁灭性问题] 终端ctrl + c终止含tail脚本后引发系统crash"
- **Root cause class**: signal/conpty teardown race.
- **Acceptance**: fuzz-style harness sends SIGINT-equivalents during active
  streaming output across 10k iterations without panic; ConPTY close path
  covered by tests in `krill-transport`.

## D2 Freeze after creating/editing a session (v2.7.0)
- **WindTerm ref**: "新建session或者对session进行属性设置后卡死"
- **Root cause class**: UI thread blocking on session-store I/O.
- **Acceptance**: session CRUD operations async off the UI thread;
  property dialog opens < 100ms with a 10k-session store (benchmark test).

## D3 ZModem/rz Chinese filename mojibake
- **WindTerm refs**: multiple rz/sz 中文文件名乱码 issues.
- **Root cause class**: no GBK↔UTF-8 filename charset negotiation.
- **Acceptance**: round-trip transfer of filenames in UTF-8, GBK and mixed
  encodings preserves names exactly (unit test fixtures per encoding).

## D4 SFTP disconnects mid-transfer of large files
- **WindTerm ref**: "从远程虚拟机下载34M的文件…ssh连接直接断开".
- **Root cause class**: transfer queue without chunked resume/backpressure.
- **Acceptance**: 1GB transfer survives injected disconnect with automatic
  resume; integration test uses a throttled local sftp fixture.

## D5 Unauditable credential storage
- **Acceptance**: all crypto via system primitives (DPAPI/CredMan, PBKDF2
  via BCrypt), zero hand-rolled crypto, storage format documented and
  schema-versioned in `docs/specs/`.
