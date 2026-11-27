# Branch `rx6800-series-x`

This branch is `fxceless/victory-fork` `main` plus two switches from TG0000. Both are off by default:

| Switch | What it does | Upstream PR |
|---|---|---|
| `VICTORY_COLD_SKIP=1` | Texture and buffer refreshes skip 2 MiB chunks that nothing has written since the last walk. | #1358 |
| `VICTORY_FRAME_HEALTH=1` or `=<file.csv>` | Writes one CSV row per frame: draws and dispatches not run (pipeline still compiling or refused), textures not on the card yet, and VRAM used against the budget. | #1415 |

`scripts/play-rx6800.ps1 -Exe <the title's executable> [-Health]` turns them on and starts the game. You need your own copy of the game; nothing of it is in this repository.

## Test machine

AMD RX 6800 16 GB (ReBAR on), Adrenalin 26.11.2, Ryzen 9 5900X, 32 GB, Windows 11, Series X profile at 1440p.

## Honest status

- The FPS figures we posted come from our local test build. That build carries more switches than this branch, some of which are not public. This branch on its own has **not** been benchmarked yet.
- `VICTORY_FRAME_HEALTH` was checked in game on the local build: 2,853 frames in a 240 s run, CSV complete, no visible change.
- Clippy passes with `-D warnings` for `x86_64-pc-windows-msvc`.
- Contributions are AI-assisted (Claude). GPL-3.0, like the rest of the repository.
