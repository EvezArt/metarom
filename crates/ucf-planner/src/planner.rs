//! ucf-planner — main planner orchestration
//!
//! Architecture:
//!   GapVector (gap.rs)           → pure diagnostic, never implies compensation
//!   Strategy + score_strategy()  → selects best execution path given gaps + policy
//!   PlanCandidate + comp_map()   → carries compensation decisions, decoupled from gap analysis
//!   build_compatibility_plan()   → materializes final CompatibilityPlan from winning candidate

use crate::gap::{analyze_gaps, GapVector};
use crate::model::{CompatibilityPlan, PlanningRequest};
use crate::plan::{
    build_compatibility_plan, default_compensation_map_for, PlanCandidate,
};
use crate::strategy::{score_strategy, ScoreWeights, Strategy};
use std::error::Error;

/// All strategies considered during planning (ordered from least to most invasive).
const CANDIDATE_STRATEGIES: &[Strategy] = &[
    Strategy::NativeBc,
    Strategy::RuntimeShim,
    Strategy::TranslateApi,
    Strategy::Emulate,
    Strategy::EmulatePlusTranslate,
    Strategy::SplitExecutionRecommended,
    Strategy::StreamingRecommended,
    Strategy::DownportRequired,
    Strategy::AugmentationRequired,
    Strategy::NotFeasible,
];

/// Entry point: produce a ranked best plan for the given request.
pub fn plan_execution(req: PlanningRequest<'_, '_, '_, '_>) -> Result<CompatibilityPlan, Box<dyn Error>> {
    let gaps = analyze_gaps(req.game, req.target);
    let helper_present = !req.helpers.is_empty();
    let weights = ScoreWeights::default();

    // 1) Score all candidate strategies against the same GapVector
    let mut scored: Vec<(Strategy, crate::model::PlanScores)> = CANDIDATE_STRATEGIES
        .iter()
        .map(|&s| {
            let scores = score_strategy(s, &gaps, req.policy, req.target, helper_present, weights);
            (s, scores)
        })
        .collect();

    // 2) Sort by total score descending; NotFeasible always last
    scored.sort_by(|(sa, a), (sb, b)| {
        if *sa == Strategy::NotFeasible { return std::cmp::Ordering::Greater; }
        if *sb == Strategy::NotFeasible { return std::cmp::Ordering::Less; }
        b.total.cmp(&a.total)
    });

    // 3) Select winner — skip strategies blocked by policy or hard gaps
    let (winning_strategy, winning_scores) = scored
        .iter()
        .find(|(s, _)| is_allowed(*s, &gaps, req.policy))
        .copied()
        .unwrap_or((Strategy::NotFeasible, crate::model::PlanScores::default()));

    // 4) Build candidate with compensation map for the winning strategy
    let mut candidate = PlanCandidate::new(winning_strategy);
    candidate.compensation_map = default_compensation_map_for(winning_strategy, &gaps);
    candidate.scores = winning_scores;
    candidate.pipeline = strategy_pipeline(winning_strategy, &gaps);
    candidate.rationale = build_rationale(winning_strategy, &gaps, &winning_scores);
    candidate.confidence = derive_confidence(&gaps, winning_scores.total);

    // 5) Apply mode-aware split/rollback preferences
    if let Some(mode_id) = req.mode_id {
        apply_mode_split_prefs(&mut candidate, req.game, mode_id);
    }

    // 6) Materialize CompatibilityPlan
    Ok(build_compatibility_plan(
        candidate,
        req.game,
        req.target,
        req.helpers,
        &gaps,
        req.mode_id,
    ))
}

// ── Strategy gate: policy + hard gap guards ──────────────────────────────────

fn is_allowed(
    strategy: Strategy,
    gaps: &GapVector,
    policy: &crate::model::PolicyProfile,
) -> bool {
    use crate::gap::GapSeverity;
    use Strategy::*;

    match strategy {
        // Streaming/split blocked by policy
        StreamingRecommended if !policy.allow_streaming => return false,
        SplitExecutionRecommended if !policy.allow_split_execution => return false,
        DownportRequired if !policy.allow_downport_classification => return false,

        // Native BC requires no hard CPU/GPU gap
        NativeBc if gaps.cpu.severity == GapSeverity::Hard => return false,
        NativeBc if gaps.gpu.severity == GapSeverity::Hard => return false,
        NativeBc if gaps.runtime.severity == GapSeverity::Hard => return false,

        // Emulation doesn't help with hard IO/network gaps
        Emulate | EmulatePlusTranslate if gaps.io.severity == GapSeverity::Hard => return false,

        _ => {}
    }

    true
}

// ── Strategy pipeline strings ─────────────────────────────────────────────────

fn strategy_pipeline(strategy: Strategy, _gaps: &GapVector) -> Vec<String> {
    match strategy {
        Strategy::NativeBc => vec![
            "native_bc_check".into(),
            "verify_equivalence".into(),
        ],
        Strategy::RuntimeShim => vec![
            "detect_runtime_mismatches".into(),
            "inject_shims".into(),
            "verify_equivalence".into(),
        ],
        Strategy::TranslateApi => vec![
            "api_translation_layer".into(),
            "shader_transpile".into(),
            "verify_equivalence".into(),
        ],
        Strategy::Emulate => vec![
            "load_emulator_core".into(),
            "map_bios_rom".into(),
            "run_emulation_loop".into(),
            "verify_equivalence".into(),
        ],
        Strategy::EmulatePlusTranslate => vec![
            "load_emulator_core".into(),
            "api_translation_layer".into(),
            "shader_transpile".into(),
            "run_emulation_loop".into(),
            "verify_equivalence".into(),
        ],
        Strategy::SplitExecutionRecommended => vec![
            "split_partition_analysis".into(),
            "launch_remote_simulation".into(),
            "launch_local_ui_audio".into(),
            "sync_state_channel".into(),
            "verify_equivalence".into(),
        ],
        Strategy::StreamingRecommended => vec![
            "setup_stream_session".into(),
            "launch_remote_render".into(),
            "stream_av_to_client".into(),
            "input_relay_channel".into(),
            "verify_equivalence".into(),
        ],
        Strategy::DownportRequired => vec![
            "downport_analysis".into(),
            "asset_reduction".into(),
            "feature_fallback_map".into(),
            "verify_equivalence".into(),
        ],
        Strategy::AugmentationRequired => vec![
            "augmentation_spec".into(),
            "hardware_probe".into(),
            "verify_equivalence".into(),
        ],
        Strategy::NotFeasible => vec![
            "feasibility_report".into(),
        ],
    }
}

// ── Rationale builder ─────────────────────────────────────────────────────────

fn build_rationale(
    strategy: Strategy,
    gaps: &GapVector,
    scores: &crate::model::PlanScores,
) -> Vec<String> {
    use crate::gap::GapSeverity;
    let mut r: Vec<String> = vec![];

    r.push(format!("Selected strategy: {:?} (total_score={})", strategy, scores.total));

    if gaps.cpu.severity != GapSeverity::None {
        r.push(format!("CPU gap [{:?}]: {:?}", gaps.cpu.severity,
            gaps.cpu.reasons.iter().map(|g| g.code.as_str()).collect::<Vec<_>>()));
    }
    if gaps.gpu.severity != GapSeverity::None {
        r.push(format!("GPU gap [{:?}]: {:?}", gaps.gpu.severity,
            gaps.gpu.reasons.iter().map(|g| g.code.as_str()).collect::<Vec<_>>()));
    }
    if gaps.runtime.severity != GapSeverity::None {
        r.push(format!("Runtime gap [{:?}]: {:?}", gaps.runtime.severity,
            gaps.runtime.reasons.iter().map(|g| g.code.as_str()).collect::<Vec<_>>()));
    }
    if gaps.timing.severity != GapSeverity::None {
        r.push(format!("Timing gap [{:?}]: {:?}", gaps.timing.severity,
            gaps.timing.reasons.iter().map(|g| g.code.as_str()).collect::<Vec<_>>()));
    }
    if gaps.legal.severity != GapSeverity::None {
        r.push(format!("Legal gap [{:?}]: {:?}", gaps.legal.severity,
            gaps.legal.reasons.iter().map(|g| g.code.as_str()).collect::<Vec<_>>()));
    }

    r
}

// ── Confidence heuristic ──────────────────────────────────────────────────────

fn derive_confidence(gaps: &GapVector, total_score: u8) -> f32 {
    use crate::gap::GapSeverity;
    let hard_count = gaps.statuses().filter(|s| s.severity == GapSeverity::Hard).count();
    let soft_count = gaps.statuses().filter(|s| s.severity == GapSeverity::Soft).count();

    let base = total_score as f32 / 100.0;
    let penalty = (hard_count as f32 * 0.15) + (soft_count as f32 * 0.05);
    (base - penalty).clamp(0.05, 0.99)
}

// ── Mode-aware split/rollback preferences ────────────────────────────────────

fn apply_mode_split_prefs(
    candidate: &mut PlanCandidate,
    game: &crate::model::GameRequirement,
    mode_id: &str,
) {
    use crate::gap::GapKind;
    use crate::plan::Compensation;

    let mode = game.fidelity_modes.iter().find(|m| m.mode_id == mode_id);
    if let Some(m) = mode {
        if let Some(split) = &m.split_execution {
            // If mode has a latency budget override, record it in rationale
            if let Some(budget) = split.latency_budget_ms_override {
                candidate.rationale.push(format!(
                    "Mode '{mode_id}' latency budget override: {budget}ms"
                ));
            }

            // Rollback window annotation
            if split.rollback_window_frames_target.is_some() {
                let target = split.rollback_window_frames_target.unwrap_or(0);
                let max = split.rollback_window_frames_max.unwrap_or(target + 4);
                candidate.rationale.push(format!(
                    "Mode '{mode_id}' rollback window: target={target} max={max} frames"
                ));
            }

            // Promote SplitExecution compensation for cpu/gpu if preferred_mode suggests it
            match split.preferred_mode.as_deref() {
                Some("split_ui_local") | Some("hybrid_prediction") => {
                    candidate.add_comp(GapKind::Cpu, Compensation::SplitExecution);
                    candidate.add_comp(GapKind::Io, Compensation::RemoteInputBridge);
                }
                Some("split_audio_local") => {
                    candidate.add_comp(GapKind::Io, Compensation::RemoteInputBridge);
                }
                _ => {}
            }
        }
    }
}
