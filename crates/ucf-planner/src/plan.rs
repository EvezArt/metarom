use crate::gap::{GapKind, GapSeverity, GapVector};
use crate::model::{
    CompatibilityPlan, Degradation, EquivalenceLevel, GameRequirement, NetworkRequirements,
    UserRequirements, VerificationTarget,
};
use crate::strategy::Strategy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Compensation {
    Emulation, ApiTranslation, RuntimeShim, TimingShim, FramePacingControl,
    InputMapper, VirtualInput, RemoteInputBridge, AssetPrefetch, FeatureFallback,
    Streaming, SplitExecution, Downport, Augmentation,
    RequiresUserSuppliedFirmware, ProbeRuntime, ManualReview,
}

pub type CompensationMap = BTreeMap<GapKind, Vec<Compensation>>;

#[derive(Debug, Clone)]
pub struct PlanCandidate {
    pub strategy: Strategy,
    pub pipeline: Vec<String>,
    pub compensation_map: CompensationMap,
    pub rationale: Vec<String>,
    pub degradations: Vec<Degradation>,
    pub user_requirements: UserRequirements,
    pub scores: crate::model::PlanScores,
    pub confidence: f32,
}

impl PlanCandidate {
    pub fn new(strategy: Strategy) -> Self {
        Self {
            strategy, pipeline: vec![], compensation_map: BTreeMap::new(),
            rationale: vec![], degradations: vec![],
            user_requirements: UserRequirements { firmware: vec![], network: NetworkRequirements::default(), setup_steps: vec![] },
            scores: crate::model::PlanScores::default(), confidence: 0.5,
        }
    }
    pub fn add_comp(&mut self, kind: GapKind, comp: Compensation) {
        self.compensation_map.entry(kind).or_default().push(comp);
    }
}

pub fn default_compensation_map_for(strategy: Strategy, gaps: &GapVector) -> CompensationMap {
    let mut map: CompensationMap = BTreeMap::new();
    let mut push = |k: GapKind, c: Compensation| { map.entry(k).or_default().push(c); };

    if gaps.cpu.severity != GapSeverity::None {
        match strategy {
            Strategy::Emulate | Strategy::EmulatePlusTranslate => push(GapKind::Cpu, Compensation::Emulation),
            Strategy::StreamingRecommended | Strategy::SplitExecutionRecommended => push(GapKind::Cpu, Compensation::SplitExecution),
            Strategy::DownportRequired => push(GapKind::Cpu, Compensation::Downport),
            Strategy::AugmentationRequired => push(GapKind::Cpu, Compensation::Augmentation),
            _ => {}
        }
    }
    if gaps.gpu.severity != GapSeverity::None {
        match strategy {
            Strategy::TranslateApi | Strategy::EmulatePlusTranslate => push(GapKind::Gpu, Compensation::ApiTranslation),
            Strategy::StreamingRecommended | Strategy::SplitExecutionRecommended => push(GapKind::Gpu, Compensation::SplitExecution),
            Strategy::DownportRequired => { push(GapKind::Gpu, Compensation::Downport); push(GapKind::Gpu, Compensation::FeatureFallback); }
            Strategy::AugmentationRequired => push(GapKind::Gpu, Compensation::Augmentation),
            _ => {}
        }
    }
    if gaps.runtime.severity != GapSeverity::None {
        match strategy {
            Strategy::RuntimeShim | Strategy::EmulatePlusTranslate => push(GapKind::Runtime, Compensation::RuntimeShim),
            Strategy::StreamingRecommended => push(GapKind::Runtime, Compensation::Streaming),
            _ => {}
        }
    }
    if gaps.timing.severity != GapSeverity::None {
        push(GapKind::Timing, Compensation::TimingShim);
        push(GapKind::Timing, Compensation::FramePacingControl);
    }
    if gaps.io.severity != GapSeverity::None {
        match strategy {
            Strategy::StreamingRecommended | Strategy::SplitExecutionRecommended => { push(GapKind::Io, Compensation::RemoteInputBridge); }
            _ => { push(GapKind::Io, Compensation::InputMapper); push(GapKind::Io, Compensation::VirtualInput); }
        }
    }
    if gaps.memory.severity != GapSeverity::None {
        match strategy {
            Strategy::StreamingRecommended | Strategy::SplitExecutionRecommended => push(GapKind::Memory, Compensation::SplitExecution),
            Strategy::DownportRequired => push(GapKind::Memory, Compensation::Downport),
            _ => push(GapKind::Memory, Compensation::AssetPrefetch),
        }
    }
    if gaps.legal.severity != GapSeverity::None {
        if gaps.legal.reasons.iter().any(|r| r.code == "USER_SUPPLIED_FIRMWARE_REQUIRED") {
            push(GapKind::Legal, Compensation::RequiresUserSuppliedFirmware);
        }
        if gaps.legal.reasons.iter().any(|r| r.code == "DRM_UNKNOWN") {
            push(GapKind::Legal, Compensation::ProbeRuntime);
            push(GapKind::Legal, Compensation::ManualReview);
        }
    }
    map
}

pub fn build_compatibility_plan(
    candidate: PlanCandidate,
    game: &GameRequirement,
    target: &crate::model::CapabilityGraph,
    helpers: &[crate::model::CapabilityGraph],
    gaps: &GapVector,
    mode_id: Option<&str>,
) -> CompatibilityPlan {
    let equivalence_min = resolve_equivalence_level(game, mode_id);
    CompatibilityPlan {
        plan_version: "0.1".into(),
        plan_id: format!("plan_{}", Uuid::new_v4().simple()),
        artifact_id: game.artifact_id.clone(),
        target_platform_id: target.platform_id.clone(),
        helper_platform_ids: helpers.iter().map(|h| h.platform_id.clone()).collect(),
        strategy: candidate.strategy.into(),
        strategy_pipeline: candidate.pipeline,
        rationale: candidate.rationale,
        gaps: gaps.summary_strings(),
        degradations: candidate.degradations,
        requirements_for_user: candidate.user_requirements,
        scores: candidate.scores,
        verification_target: VerificationTarget {
            equivalence_min,
            test_profile: "smoke_plus_input_latency".into(),
        },
        confidence: candidate.confidence,
    }
}

fn resolve_equivalence_level(game: &GameRequirement, mode_id: Option<&str>) -> EquivalenceLevel {
    if let Some(id) = mode_id {
        if let Some(m) = game.fidelity_modes.iter().find(|m| m.mode_id == id) {
            return m.acceptable_equivalence_min.clone();
        }
    }
    game.fidelity_modes.first()
        .map(|m| m.acceptable_equivalence_min.clone())
        .unwrap_or(EquivalenceLevel::L2_INTERACTIVE)
}
