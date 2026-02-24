# MetaROM

**Universal game runtime + compatibility planner.**

MetaROM treats emulator cores as arcade cart modules and games as ROM artifacts,
then uses the UCF (Universal Compatibility Fabric) planner to determine the best
execution strategy for any artifact on any target platform.

## Repo layout

```
metarom/
├── schemas/
│   ├── capability_graph.schema.json   # Target platform descriptor
│   ├── game_requirement.schema.json   # Game/artifact requirements
│   └── sts_profile.schema.json        # Semantic Tick Scaling profile
├── crates/
│   ├── ucf-planner/                   # UCF planning engine (CLI + library)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── main.rs
│   │       ├── model.rs               # Data types (stub — fill from spec)
│   │       ├── gap.rs                 # Pure gap diagnostic
│   │       ├── strategy.rs            # Strategy scoring
│   │       ├── plan.rs                # PlanCandidate + compensation
│   │       ├── planner.rs             # Orchestration
│   │       └── cli.rs                 # CLI runner
│   └── mrom-ecore-abi/                # C-ABI for .mrom arcade cart modules
│       └── src/lib.rs
└── examples/
    └── ps2_to_pc.json                 # Example planning request
```

## Quick start

```bash
# Build the planner
cargo build -p ucf-planner

# Run a plan (supply your own cap/req JSON files)
ucf-planner plan --artifact examples/game_req.json --target examples/pc_cap.json

# Use a specific fidelity mode
ucf-planner plan --artifact examples/game_req.json --target examples/ps2_cap.json --mode gameplay
```

## Architecture

- **GapVector** — pure diagnostic; never implies compensation
- **Strategy** — scored against gaps + policy to select execution path
- **PlanCandidate** — carries compensation map, mode-aware rollback prefs
- **CompatibilityPlan** — output artifact with pipeline, rationale, scores, verification target

## License

AGPL-3.0 (community/free tier). Commercial licenses available — see [EVEZ OS](https://evez-autonomizer.vercel.app).
