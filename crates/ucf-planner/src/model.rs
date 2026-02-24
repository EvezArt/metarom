//! model.rs — UCF shared data types (stub)
//! 
//! TODO: expand from the full UCF v0.1 spec.
//! All types must be kept in sync with the JSON schemas in /schemas/.

use serde::{Deserialize, Serialize};

// ── Re-exported from schemas ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityGraph {
    pub capability_version: String,
    pub platform_id: String,
    pub label: String,
    pub class: String,
    pub host_os: HostOs,
    pub cpu: CpuCapability,
    pub gpu: GpuCapability,
    pub memory: MemoryCapability,
    pub io: IoCapability,
    pub timing: TimingCapability,
    pub security: SecurityCapability,
    pub legal: LegalCapability,
    pub profiles: ProfilesMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostOs { pub family: String, pub version: String, pub abi: Vec<String>, pub syscalls: Vec<String> }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuCapability { pub isas: Vec<String>, pub cores: u32, pub threads: u32, pub clock_mhz: f64, pub simd: Vec<String>, pub features: serde_json::Value }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuCapability { pub apis: Vec<String>, pub shader_models: Vec<String>, pub features: serde_json::Value, pub vram_mb: u32, pub throughput_hint: serde_json::Value }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryCapability { pub ram_mb: u32, pub bandwidth_gbps: f64, pub storage: StorageCapability }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageCapability { pub internal_mb: u32, pub streaming_read_mbps: f64, pub seek_latency_ms: f64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IoCapability { pub inputs: Vec<String>, pub audio_out: bool, pub video_out: Vec<String>, pub network: NetworkCapability }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkCapability { pub available: bool, pub bandwidth_mbps: Option<f64>, pub rtt_ms: Option<f64>, pub jitter_ms: Option<f64> }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingCapability { pub display_modes_hz: Vec<f64>, pub timer_resolution_us: u32, pub interrupt_model: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityCapability { pub unsigned_code_allowed: bool, pub external_coprocessor_support: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegalCapability { pub firmware_required: bool, pub redistributable_firmware: bool }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilesMeta { pub measured: bool, pub source: String }

// ── Game requirement ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameRequirement {
    pub requirement_version: String,
    pub artifact_id: String,
    pub kind: String,
    pub source_type: String,
    pub targets_original: Vec<String>,
    pub cpu: CpuRequirement,
    pub gpu: GpuRequirement,
    pub memory: MemoryRequirement,
    pub runtime: RuntimeRequirement,
    pub io: IoRequirement,
    pub timing: TimingRequirement,
    pub fidelity_modes: Vec<FidelityMode>,
    pub extractor_confidence: ExtractorConfidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuRequirement {
    pub required_isa: Vec<String>,
    pub min_cores: u32,
    pub threading_model: String,
    pub simd_required: Option<Vec<String>>,
    pub perf_budget_hint: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuRequirement {
    pub required_apis: Vec<String>,
    pub shader_model: Option<String>,
    pub features_required: serde_json::Value,
    pub vram_min_mb: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRequirement {
    pub ram_min_mb: u32,
    pub storage_install_mb: Option<u32>,
    pub streaming_read_mbps: Option<f64>,
    pub seek_tolerance_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeRequirement {
    pub os_families: Vec<String>,
    pub syscalls_or_apis: Option<Vec<String>>,
    pub middleware: Option<Vec<String>>,
    pub anti_cheat: Option<bool>,
    pub drm: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IoRequirement { pub required_inputs: Vec<String>, pub online_required: bool }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingRequirement {
    pub target_fps: Option<f64>,
    pub frame_pacing_sensitive: Option<bool>,
    pub simulation_tick_hz: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FidelityMode {
    pub mode_id: String,
    pub priority: String,
    pub acceptable_equivalence_min: EquivalenceLevel,
    pub split_execution: Option<SplitExecutionPrefs>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitExecutionPrefs {
    pub preferred_mode: Option<String>,
    pub rollback_window_frames_min: Option<u32>,
    pub rollback_window_frames_target: Option<u32>,
    pub rollback_window_frames_max: Option<u32>,
    pub latency_budget_ms_override: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EquivalenceLevel {
    L0_BOOT, L1_STABLE, L2_INTERACTIVE, L3_GAMEPLAY_EQ, L4_RENDER_EQ, L5_BIT_EXACT,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractorConfidence { pub static_analysis: f32, pub runtime_probe: f32, pub trace_inference: f32 }

// ── Policy ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyProfile {
    pub policy_version: String,
    pub profile_id: String,
    pub latency_budget_ms: f64,
    pub min_fidelity_score: u8,
    pub max_legal_risk: u8,
    pub prefer_local_execution: bool,
    pub allow_streaming: bool,
    pub allow_split_execution: bool,
    pub allow_downport_classification: bool,
    pub allow_unverified_plans: bool,
}

// ── Plan output ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityPlan {
    pub plan_version: String,
    pub plan_id: String,
    pub artifact_id: String,
    pub target_platform_id: String,
    pub helper_platform_ids: Vec<String>,
    pub strategy: StrategyClass,
    pub strategy_pipeline: Vec<String>,
    pub rationale: Vec<String>,
    pub gaps: GapSummary,
    pub degradations: Vec<Degradation>,
    pub requirements_for_user: UserRequirements,
    pub scores: PlanScores,
    pub verification_target: VerificationTarget,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StrategyClass {
    NativeBc, Emulate, TranslateApi, RuntimeShim, EmulatePlusTranslate,
    DownportRequired, StreamingRecommended, SplitExecutionRecommended,
    AugmentationRequired, NotFeasible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapSummary {
    pub cpu_gap: String, pub gpu_gap: String, pub memory_gap: String,
    pub runtime_gap: String, pub timing_gap: String,
    pub io_gap: Option<String>, pub legal_gap: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Degradation { pub subsystem: String, pub description: String, pub equivalence_impact: String }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserRequirements {
    pub firmware: Vec<String>,
    pub network: NetworkRequirements,
    pub setup_steps: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkRequirements {
    pub min_bandwidth_mbps: Option<f64>,
    pub max_rtt_ms: Option<f64>,
}

#[derive(Debug, Clone, Default, Copy, Serialize, Deserialize)]
pub struct PlanScores {
    pub fidelity: u8, pub latency: u8, pub engineering_effort: u8,
    pub runtime_cost: u8, pub legal_risk: u8, pub determinism: u8,
    pub user_friction: u8, pub total: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationTarget { pub equivalence_min: EquivalenceLevel, pub test_profile: String }

// ── PlanningRequest (lifetime-parametric) ────────────────────────────────────

pub struct PlanningRequest<'game, 'target, 'helpers, 'policy> {
    pub game: &'game GameRequirement,
    pub target: &'target CapabilityGraph,
    pub helpers: &'helpers [CapabilityGraph],
    pub policy: &'policy PolicyProfile,
    pub mode_id: Option<&'game str>,
}
