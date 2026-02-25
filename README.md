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
│   ├── capability_graph.schema.json   # Target platform descriptor
│   ├── game_requirement.schema.json   # Game/artifact requirements
│   ├── sts_profile.schema.json        # Semantic Tick Scaling profile
│   └── training_record.schema.json    # .mrom.train.json schema (Phase 4)
├── crates/
│   ├── gb-core/                       # Game Boy emulator core
│   │   └── src/
│   │       ├── lib.rs                 # Phase 4: PPU+OAM+Window, APU channels, training
│   │       └── bin/
│   │           ├── letsplay.rs        # ASCII frame renderer
│   │           └── letsplay_train.rs  # Training data extractor (Phase 4)
│   ├── ucf-planner/                   # UCF planning engine (CLI + library)
│   └── mrom-ecore-abi/                # C-ABI for .mrom arcade cart modules
└── examples/
    └── ps2_to_pc_req.json             # Example planning request
```

## Phase 4 Features

- **OAM sprite rendering** — 40 sprite limit, 8x8/8x16 modes, X/Y flip, BG priority, per-sprite palette
- **Window layer** — WY/WX registers, internal line counter (wlc), correct window tile map select
- **Full palette registers** — BGP (FF47), OBP0 (FF48), OBP1 (FF49)
- **APU channels** — Square1, Square2, Wave, Noise with per-channel sample output
- **APU sample buffer** — 48kHz stereo PCM, drain_samples() API
- **Training extractor** — `letsplay_train` binary plays any ROM → `.mrom.train.json`
- **Training schema** — `schemas/training_record.schema.json` defines the v1 record format

## Quick start

```bash
# Build all crates
cargo build

# Play synthetic ROM (ASCII output, 10 frames)
cargo run --bin letsplay -- 10

# Extract training data (60 frames → JSON)
cargo run --bin letsplay_train -- 60 output.mrom.train.json

# Run a plan (UCF planner)
cargo run -p ucf-planner -- plan --artifact examples/game_req.json --target examples/pc_cap.json
```

## Training pipeline

Every ROM run produces a `.mrom.train.json` file:

```json
{
  "version": "mrom.train.v1",
  "rom_title": "EVEZ-OS-TRAIN",
  "epoch": "gen1_nes",
  "total_frames": 60,
  "frames": [
    { "frame": 0, "t_cycles": 70224, "pc": 352, "ly": 144, "ppu_mode": 1, ... },
    ...
  ]
}
```

Epoch classification:
- `gen1_nes` — DMG cartridges (non-CGB)
- `gen2_snes_genesis` — CGB cartridges

These feed directly into the EVEZ-OS `console_war_trainer` for epoch progression.

## Architecture

- **GapVector** — pure diagnostic; never implies compensation
- **Strategy** — scored against gaps + policy to select execution path
- **PlanCandidate** — carries compensation map, mode-aware rollback prefs
- **CompatibilityPlan** — output artifact with pipeline, rationale, scores

## License

AGPL-3.0 (community/free tier). Commercial licenses available — see [EVEZ OS](https://evez-autonomizer.vercel.app).
