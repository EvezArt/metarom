# MetaROM

**Universal game runtime + compatibility planner.**

MetaROM treats emulator cores as arcade cart modules and games as ROM artifacts,
then uses the UCF (Universal Compatibility Fabric) planner to determine the best
execution strategy for any artifact on any target platform.

Every ROM that runs through the emulator becomes a training file. That's the law.

## Repo layout

```
metarom/
├── schemas/
│   ├── capability_graph.schema.json
│   ├── game_requirement.schema.json
│   ├── sts_profile.schema.json
│   └── training_record.schema.json    # .mrom.train.json schema (Phase 4+)
├── crates/
│   ├── gb-core/                       # Game Boy emulator core
│   │   └── src/
│   │       ├── lib.rs                 # Phase 5: full SM83, APU HW timers, training
│   │       └── bin/
│   │           ├── letsplay.rs        # ASCII frame renderer
│   │           ├── letsplay_train.rs  # Single-ROM training extractor
│   │           └── letsplay_batch.rs  # Batch ROM-to-training runner (Phase 5)
│   ├── ucf-planner/                   # UCF planning engine
│   └── mrom-ecore-abi/                # C-ABI for .mrom arcade cart modules
└── examples/
    └── ps2_to_pc_req.json
```

## Phase 5 Features

- **Full SM83 instruction set** — all 251 opcodes correctly implemented via `exec_op()`
  - Complete LD r8/r8 grid (64 instructions, 0x40–0x7F)
  - Full ALU grid: ADD/ADC/SUB/SBC/AND/XOR/OR/CP with r8 and d8 operands (0x80–0xBF, 0xC6–0xFE)
  - All flag-correct arithmetic: half-carry, carry, zero, subtract
  - **DAA** (BCD correction) — complete implementation
  - **PUSH/POP** all register pairs including AF (with F lower nibble mask)
  - Conditional jumps: JP cc, JR cc with taken/not-taken cycle counts
  - Conditional CALL cc (24 cycles taken, 12 not-taken)
  - Conditional RET cc (20 cycles taken, 8 not-taken)
  - RST handlers, LDH, LD (C)/A, ADD SP/e8, LD HL SP+e8
- **APU frame sequencer** — 512Hz hardware timer (every 8192 cycles)
  - Length counter: clocks at steps 0, 2, 4, 6 — disables channel at zero
  - Frequency sweep (Square1 NR10): clocks at steps 2, 6
  - Envelope: clocks at step 7 — volume ramp up/down per channel
  - sweep_shadow register, sweep_enabled, sweep_timer
- **Batch ROM runner** — `letsplay_batch` processes entire ROM directories
  - One `.mrom.train.json` per ROM, written to output directory
  - `batch_manifest.json` — summary of all runs (title, epoch, frames, ok/fail)
  - Handles .gb, .gbc, .rom extensions
  - Graceful error handling per ROM (bad ROM doesn't stop batch)

## Quick start

```bash
# Build all
cargo build

# Single ROM training (60 frames)
cargo run --bin letsplay_train -- 60 output.mrom.train.json

# Batch: process every .gb/.gbc in roms/ → training_output/
cargo run --bin letsplay_batch -- roms/ training_output/ 300

# ASCII renderer
cargo run --bin letsplay -- 10

# UCF planner
cargo run -p ucf-planner -- plan --artifact examples/game_req.json --target examples/pc_cap.json
```

## Training pipeline

```
roms/
├── tetris.gb      ──→  training_output/tetris.mrom.train.json
├── zelda.gb       ──→  training_output/zelda.mrom.train.json
└── pokemon.gb     ──→  training_output/pokemon.mrom.train.json
                         training_output/batch_manifest.json

Each .mrom.train.json feeds into EVEZ-OS console_war_trainer for epoch progression.
```

Epoch classification:
- `gen1_nes` — DMG cartridges (non-CGB)
- `gen2_snes_genesis` — CGB cartridges

## Architecture

- **GapVector** — pure diagnostic; never implies compensation
- **Strategy** — scored against gaps + policy to select execution path
- **PlanCandidate** — carries compensation map, mode-aware rollback prefs
- **CompatibilityPlan** — output artifact with pipeline, rationale, scores

## License

AGPL-3.0 (community/free tier). Commercial licenses available — see [EVEZ OS](https://evez-autonomizer.vercel.app).
