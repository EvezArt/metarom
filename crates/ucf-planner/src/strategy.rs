use crate::gap::{GapSeverity, GapVector};
use crate::model::{CapabilityGraph, PlanScores, PolicyProfile, StrategyClass};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Strategy {
    NativeBc,
    Emulate,
    TranslateApi,
    RuntimeShim,
    EmulatePlusTranslate,
    DownportRequired,
    StreamingRecommended,
    SplitExecutionRecommended,
    AugmentationRequired,
    NotFeasible,
}

impl From<Strategy> for StrategyClass {
    fn from(value: Strategy) -> Self {
        match value {
            Strategy::NativeBc => StrategyClass::NativeBc,
            Strategy::Emulate => StrategyClass::Emulate,
            Strategy::TranslateApi => StrategyClass::TranslateApi,
            Strategy::RuntimeShim => StrategyClass::RuntimeShim,
            Strategy::EmulatePlusTranslate => StrategyClass::EmulatePlusTranslate,
            Strategy::DownportRequired => StrategyClass::DownportRequired,
            Strategy::StreamingRecommended => StrategyClass::StreamingRecommended,
            Strategy::SplitExecutionRecommended => StrategyClass::SplitExecutionRecommended,
            Strategy::AugmentationRequired => StrategyClass::AugmentationRequired,
            Strategy::NotFeasible => StrategyClass::NotFeasible,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScoreWeights {
    pub fidelity: u16,
    pub latency: u16,
    pub engineering_effort_inverted: u16,
    pub runtime_cost_inverted: u16,
    pub legal_risk_inverted: u16,
    pub determinism: u16,
    pub user_friction_inverted: u16,
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            fidelity: 22, latency: 20, engineering_effort_inverted: 12,
            runtime_cost_inverted: 10, legal_risk_inverted: 12,
            determinism: 14, user_friction_inverted: 10,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct WorkingAxes {
    fidelity: i32, latency: i32, engineering_effort: i32,
    runtime_cost: i32, legal_risk: i32, determinism: i32, user_friction: i32,
}

impl WorkingAxes {
    fn baseline() -> Self {
        Self { fidelity: 80, latency: 80, engineering_effort: 50,
               runtime_cost: 50, legal_risk: 10, determinism: 70, user_friction: 30 }
    }

    fn to_plan_scores(self, weights: ScoreWeights) -> PlanScores {
        let mut s = PlanScores {
            fidelity: clamp(self.fidelity), latency: clamp(self.latency),
            engineering_effort: clamp(self.engineering_effort),
            runtime_cost: clamp(self.runtime_cost), legal_risk: clamp(self.legal_risk),
            determinism: clamp(self.determinism), user_friction: clamp(self.user_friction),
            total: 0,
        };
        let total_i =
            (s.fidelity as i32 * weights.fidelity as i32) +
            (s.latency as i32 * weights.latency as i32) +
            ((100 - s.engineering_effort as i32) * weights.engineering_effort_inverted as i32) +
            ((100 - s.runtime_cost as i32) * weights.runtime_cost_inverted as i32) +
            ((100 - s.legal_risk as i32) * weights.legal_risk_inverted as i32) +
            (s.determinism as i32 * weights.determinism as i32) +
            ((100 - s.user_friction as i32) * weights.user_friction_inverted as i32);
        let max_total = 100 * (weights.fidelity as i32 + weights.latency as i32
            + weights.engineering_effort_inverted as i32 + weights.runtime_cost_inverted as i32
            + weights.legal_risk_inverted as i32 + weights.determinism as i32
            + weights.user_friction_inverted as i32);
        s.total = clamp(((total_i as f32 / max_total as f32) * 100.0).round() as i32);
        s
    }
}

pub fn score_strategy(
    strategy: Strategy, gaps: &GapVector, policy: &PolicyProfile,
    _target: &CapabilityGraph, helper_present: bool, weights: ScoreWeights,
) -> PlanScores {
    let mut a = WorkingAxes::baseline();
    penalize(&mut a.fidelity, gaps.cpu.severity, 10, 25);
    penalize(&mut a.fidelity, gaps.gpu.severity, 8, 20);
    penalize(&mut a.fidelity, gaps.memory.severity, 5, 15);
    penalize(&mut a.determinism, gaps.timing.severity, 8, 20);
    a.legal_risk += severity_risk_delta(gaps.legal.severity);
    a.legal_risk += severity_risk_delta(gaps.runtime.severity);

    match strategy {
        Strategy::NativeBc => { a.fidelity += 10; a.latency += 10; a.engineering_effort += 5; a.runtime_cost += 5; a.determinism += 5; }
        Strategy::Emulate => { a.fidelity += 5; a.latency -= 15; a.runtime_cost -= 20; a.determinism += 5; }
        Strategy::TranslateApi => { a.fidelity -= 5; a.latency -= 5; a.engineering_effort -= 10; a.runtime_cost -= 10; a.determinism -= 5; }
        Strategy::RuntimeShim => { a.engineering_effort -= 15; a.determinism -= 10; }
        Strategy::EmulatePlusTranslate => { a.latency -= 20; a.engineering_effort -= 20; a.runtime_cost -= 25; a.determinism -= 5; }
        Strategy::DownportRequired => { a.fidelity -= 15; a.engineering_effort -= 35; a.user_friction += 10; }
        Strategy::StreamingRecommended => {
            a.fidelity -= 10; a.latency -= 25; a.runtime_cost -= 10;
            a.determinism -= 20; a.user_friction -= 10;
            if helper_present { a.latency += 5; }
        }
        Strategy::SplitExecutionRecommended => {
            a.fidelity -= 5; a.latency -= 15; a.engineering_effort -= 25;
            a.runtime_cost -= 15; a.determinism -= 15; a.user_friction -= 10;
            if helper_present { a.latency += 8; }
        }
        Strategy::AugmentationRequired => { a.engineering_effort -= 30; a.runtime_cost -= 20; a.user_friction -= 25; }
        Strategy::NotFeasible => {
            a = WorkingAxes { fidelity: 0, latency: 0, engineering_effort: 100,
                runtime_cost: 100, legal_risk: 100, determinism: 0, user_friction: 100 };
        }
    }

    if policy.prefer_local_execution {
        match strategy {
            Strategy::StreamingRecommended | Strategy::SplitExecutionRecommended => {
                a.latency -= 5; a.user_friction += 5;
            }
            _ => {}
        }
    }
    a.to_plan_scores(weights)
}

fn penalize(field: &mut i32, sev: GapSeverity, soft_penalty: i32, hard_penalty: i32) {
    match sev { GapSeverity::None => {} GapSeverity::Soft => *field -= soft_penalty, GapSeverity::Hard => *field -= hard_penalty, }
}
fn severity_risk_delta(sev: GapSeverity) -> i32 {
    match sev { GapSeverity::None => 0, GapSeverity::Soft => 20, GapSeverity::Hard => 40, }
}
fn clamp(v: i32) -> u8 { if v < 0 { 0 } else if v > 100 { 100 } else { v as u8 } }
