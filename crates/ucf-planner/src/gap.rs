use crate::model::{CapabilityGraph, GameRequirement};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GapSeverity {
    None,
    Soft,
    Hard,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GapKind {
    Cpu, Gpu, Memory, Runtime, Io, Timing, Legal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapReason {
    pub code: String,
    pub detail: Option<String>,
}

impl GapReason {
    pub fn new(code: impl Into<String>) -> Self {
        Self { code: code.into(), detail: None }
    }
    pub fn with_detail(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { code: code.into(), detail: Some(detail.into()) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapStatus {
    pub severity: GapSeverity,
    pub reasons: Vec<GapReason>,
}

impl GapStatus {
    pub fn none() -> Self { Self { severity: GapSeverity::None, reasons: vec![] } }
    pub fn soft(reason: GapReason) -> Self { Self { severity: GapSeverity::Soft, reasons: vec![reason] } }
    pub fn hard(reason: GapReason) -> Self { Self { severity: GapSeverity::Hard, reasons: vec![reason] } }
    pub fn push_reason(&mut self, reason: GapReason) {
        self.reasons.push(reason);
        if !self.reasons.is_empty() && self.severity == GapSeverity::None {
            self.severity = GapSeverity::Soft;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapVector {
    pub cpu: GapStatus,
    pub gpu: GapStatus,
    pub memory: GapStatus,
    pub runtime: GapStatus,
    pub io: GapStatus,
    pub timing: GapStatus,
    pub legal: GapStatus,
}

impl GapVector {
    pub fn has_any_hard(&self) -> bool {
        self.statuses().any(|s| s.severity == GapSeverity::Hard)
    }
    pub fn hardest(&self) -> GapSeverity {
        self.statuses().map(|s| s.severity).max().unwrap_or(GapSeverity::None)
    }
    pub fn statuses(&self) -> impl Iterator<Item = &GapStatus> {
        [&self.cpu, &self.gpu, &self.memory, &self.runtime, &self.io, &self.timing, &self.legal].into_iter()
    }
    pub fn summary_strings(&self) -> crate::model::GapSummary {
        crate::model::GapSummary {
            cpu_gap: format!("{:?}", self.cpu.severity).to_lowercase(),
            gpu_gap: format!("{:?}", self.gpu.severity).to_lowercase(),
            memory_gap: format!("{:?}", self.memory.severity).to_lowercase(),
            runtime_gap: format!("{:?}", self.runtime.severity).to_lowercase(),
            timing_gap: format!("{:?}", self.timing.severity).to_lowercase(),
            io_gap: Some(format!("{:?}", self.io.severity).to_lowercase()),
            legal_gap: Some(format!("{:?}", self.legal.severity).to_lowercase()),
        }
    }
}

pub fn analyze_gaps(game: &GameRequirement, target: &CapabilityGraph) -> GapVector {
    GapVector {
        cpu: analyze_cpu_gap(game, target),
        gpu: analyze_gpu_gap(game, target),
        memory: analyze_memory_gap(game, target),
        runtime: analyze_runtime_gap(game, target),
        io: analyze_io_gap(game, target),
        timing: analyze_timing_gap(game, target),
        legal: analyze_legal_gap(game, target),
    }
}

fn analyze_cpu_gap(game: &GameRequirement, target: &CapabilityGraph) -> GapStatus {
    let isa_match = game.cpu.required_isa.iter().any(|req| {
        target.cpu.isas.iter().any(|isa| isa.eq_ignore_ascii_case(req))
    });
    if !isa_match {
        return GapStatus::hard(GapReason::new("ISA_MISMATCH"));
    }
    if target.cpu.cores < game.cpu.min_cores {
        return GapStatus::soft(GapReason::with_detail(
            "CPU_CORE_COUNT_LOW",
            format!("need {}, have {}", game.cpu.min_cores, target.cpu.cores),
        ));
    }
    if let Some(simd_req) = &game.cpu.simd_required {
        let simd_ok = simd_req.iter().all(|r| {
            target.cpu.simd.iter().any(|s| s.eq_ignore_ascii_case(r))
        });
        if !simd_ok {
            return GapStatus::soft(GapReason::new("SIMD_FEATURE_GAP"));
        }
    }
    GapStatus::none()
}

fn analyze_gpu_gap(game: &GameRequirement, target: &CapabilityGraph) -> GapStatus {
    let api_match = game.gpu.required_apis.iter().any(|req_api| {
        target.gpu.apis.iter().any(|api| api.eq_ignore_ascii_case(req_api))
    });
    let shader_model_ok = match &game.gpu.shader_model {
        Some(sm) => target.gpu.shader_models.iter().any(|m| m.eq_ignore_ascii_case(sm)),
        None => true,
    };
    let vram_ok = game.gpu.vram_min_mb.map(|v| target.gpu.vram_mb >= v).unwrap_or(true);
    if !api_match && !shader_model_ok {
        return GapStatus::hard(GapReason::new("GPU_API_AND_SHADER_MODEL_MISMATCH"));
    }
    if !api_match || !shader_model_ok || !vram_ok {
        let mut g = GapStatus::soft(GapReason::new("GPU_FEATURE_GAP"));
        if !vram_ok { g.push_reason(GapReason::new("VRAM_BELOW_MIN")); }
        return g;
    }
    GapStatus::none()
}

fn analyze_memory_gap(game: &GameRequirement, target: &CapabilityGraph) -> GapStatus {
    if target.memory.ram_mb < game.memory.ram_min_mb {
        return GapStatus::hard(GapReason::with_detail(
            "RAM_BELOW_MIN",
            format!("need {}MB, have {}MB", game.memory.ram_min_mb, target.memory.ram_mb),
        ));
    }
    let mut soft = GapStatus::none();
    if let Some(req_mbps) = game.memory.streaming_read_mbps {
        if target.memory.storage.streaming_read_mbps < req_mbps {
            soft = GapStatus::soft(GapReason::new("STREAM_READ_BANDWIDTH_LOW"));
        }
    }
    if let Some(seek_tol) = game.memory.seek_tolerance_ms {
        if target.memory.storage.seek_latency_ms > seek_tol {
            if soft.severity == GapSeverity::None {
                soft = GapStatus::soft(GapReason::new("STORAGE_SEEK_TOO_HIGH"));
            } else {
                soft.push_reason(GapReason::new("STORAGE_SEEK_TOO_HIGH"));
            }
        }
    }
    soft
}

fn analyze_runtime_gap(game: &GameRequirement, target: &CapabilityGraph) -> GapStatus {
    let os_match = game.runtime.os_families.iter().any(|os| os.eq_ignore_ascii_case(&target.host_os.family));
    if !os_match { return GapStatus::hard(GapReason::new("OS_FAMILY_MISMATCH")); }
    if game.runtime.anti_cheat.unwrap_or(false) {
        return GapStatus::soft(GapReason::new("ANTI_CHEAT_POTENTIAL_BLOCKER"));
    }
    GapStatus::none()
}

fn analyze_io_gap(game: &GameRequirement, target: &CapabilityGraph) -> GapStatus {
    let missing: Vec<String> = game.io.required_inputs.iter()
        .filter(|req| !target.io.inputs.iter().any(|i| i.eq_ignore_ascii_case(req)))
        .cloned().collect();
    if !missing.is_empty() {
        return GapStatus::soft(GapReason::with_detail("MISSING_INPUTS", format!("{missing:?}")));
    }
    if game.io.online_required && !target.io.network.available {
        return GapStatus::hard(GapReason::new("ONLINE_REQUIRED_NETWORK_UNAVAILABLE"));
    }
    GapStatus::none()
}

fn analyze_timing_gap(game: &GameRequirement, target: &CapabilityGraph) -> GapStatus {
    if game.timing.frame_pacing_sensitive.unwrap_or(false) && target.timing.timer_resolution_us > 5_000 {
        return GapStatus::soft(GapReason::new("TIMER_RESOLUTION_COARSE"));
    }
    if let Some(target_fps) = game.timing.target_fps {
        let close_mode = target.timing.display_modes_hz.iter().any(|hz| (*hz - target_fps).abs() < 2.0);
        if !close_mode { return GapStatus::soft(GapReason::new("DISPLAY_MODE_MISMATCH")); }
    }
    GapStatus::none()
}

fn analyze_legal_gap(game: &GameRequirement, target: &CapabilityGraph) -> GapStatus {
    if target.legal.firmware_required && !target.legal.redistributable_firmware {
        return GapStatus::soft(GapReason::new("USER_SUPPLIED_FIRMWARE_REQUIRED"));
    }
    if game.runtime.drm.as_deref() == Some("unknown") {
        return GapStatus::soft(GapReason::new("DRM_UNKNOWN"));
    }
    GapStatus::none()
}