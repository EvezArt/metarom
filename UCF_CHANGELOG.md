# UCF v0.2 — Architecture Change Log

## v0.2 (2026-02-24)

### Breaking Changes
- `GapVector` is now **pure diagnostic only** — no compensation fields on GapStatus or SubsystemGap
- `CompensationMap` (`BTreeMap<GapKind, Vec<Compensation>>`) lives on `PlanCandidate` in `plan.rs`
- Per-mode `split_execution` block added to `fidelity_modes[]` in `game_requirement.schema.json`

### New
- `schemas/sts_profile.schema.json` — Semantic Tick Scaling Profile promoted to first-class schema
- `crates/ucf-planner/src/strategy.rs` — Strategy enum + ScoreWeights + `score_strategy()` extracted from planner
- `crates/ucf-planner/src/plan.rs` — `PlanCandidate`, `CompensationMap`, `Compensation` enum, `default_compensation_map_for()`, `build_compatibility_plan()`
- `crates/ucf-planner/src/gap.rs` — Pure diagnostic: `GapVector`, `GapStatus`, `GapReason`, `GapSeverity`, `GapKind`, `analyze_gaps()`

### Architecture

```
analyze_gaps(game, target)  →  GapVector  (DIAGNOSTIC ONLY)
                                    ↓
for each Strategy candidate:
  score_strategy(strategy, gaps, policy, target, helper_present, weights)  →  PlanScores
  default_compensation_map_for(strategy, gaps)  →  CompensationMap
  PlanCandidate { strategy, compensation_map, scores, ... }
                                    ↓
build_compatibility_plan(best_candidate, ...)  →  CompatibilityPlan (JSON output)
```

### Why
- Gap model = stable, unit-testable, reusable for analytics without repair intent
- Compensation lives on candidate = planner can compare multiple repair strategies against the same GapVector
- STS first-class = unlocks 'newer on older' paths without pretending everything is a demake
- Per-mode rollback window = competitive vs archival fidelity modes can diverge cleanly

## v0.1 (initial)
- JSON schemas: capability_graph, game_requirement, compatibility_plan, policy_profile, tolerance_profile
- Rust planner skeleton: model, gap (with compensation), score, planner
