use crate::model::{CapabilityGraph, GameRequirement, PlanningRequest, PolicyProfile};
use crate::planner::plan_execution;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

pub fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 { print_help(); std::process::exit(2); }
    match args[1].as_str() {
        "plan" => run_plan(&args[2..]),
        _ => { eprintln!("unknown command: {}", args[1]); print_help(); std::process::exit(2); }
    }
}

fn run_plan(args: &[String]) -> Result<(), Box<dyn Error>> {
    let mut artifact_path: Option<PathBuf> = None;
    let mut target_path: Option<PathBuf> = None;
    let mut helper_paths: Vec<PathBuf> = vec![];
    let mut policy_path: Option<PathBuf> = None;
    let mut mode_id: Option<String> = None;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--artifact" => { i += 1; artifact_path = Some(PathBuf::from(require_arg(args, i, "--artifact")?)); }
            "--target"   => { i += 1; target_path = Some(PathBuf::from(require_arg(args, i, "--target")?)); }
            "--helper"   => { i += 1; helper_paths.push(PathBuf::from(require_arg(args, i, "--helper")?)); }
            "--policy"   => { i += 1; policy_path = Some(PathBuf::from(require_arg(args, i, "--policy")?)); }
            "--mode"     => { i += 1; mode_id = Some(require_arg(args, i, "--mode")?.to_string()); }
            other => { return Err(format!("unexpected argument: {other}").into()); }
        }
        i += 1;
    }

    let artifact_path = artifact_path.ok_or("missing --artifact <req.json>")?;
    let target_path = target_path.ok_or("missing --target <cap.json>")?;
    let game: GameRequirement = read_json(&artifact_path)?;
    let target: CapabilityGraph = read_json(&target_path)?;
    let helpers: Vec<CapabilityGraph> = helper_paths.iter().map(read_json).collect::<Result<Vec<_>, _>>()?;
    let policy: PolicyProfile = if let Some(p) = policy_path { read_json(&p)? } else { default_policy() };

    let req = PlanningRequest { game: &game, target: &target, helpers: &helpers, policy: &policy, mode_id: mode_id.as_deref() };
    let plan = plan_execution(req)?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<T, Box<dyn Error>> {
    let s = fs::read_to_string(path)?;
    Ok(serde_json::from_str::<T>(&s)?)
}

fn require_arg<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, Box<dyn Error>> {
    args.get(idx).map(|s| s.as_str()).ok_or_else(|| format!("missing value for {flag}").into())
}

fn default_policy() -> PolicyProfile {
    PolicyProfile {
        policy_version: "0.1".into(), profile_id: "default_playable".into(),
        latency_budget_ms: 60.0, min_fidelity_score: 40, max_legal_risk: 70,
        prefer_local_execution: true, allow_streaming: true, allow_split_execution: true,
        allow_downport_classification: true, allow_unverified_plans: false,
    }
}

fn print_help() {
    eprintln!("\
ucf-planner <command>

Commands:
  plan --artifact <req.json> --target <cap.json> [--helper <cap.json> ...] [--policy <policy.json>] [--mode <mode_id>]

Examples:
  ucf-planner plan --artifact game_req.json --target ps2_cap.json --helper pc_cap.json
  ucf-planner plan --artifact game_req.json --target win11_cap.json --mode baseline
");
}
