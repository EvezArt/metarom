// ── Ranked planning: returns top-3 candidates ────────────────────────────────

/// Result carrying the winning plan plus up to 2 runner-up candidates.
#[derive(Debug)]
pub struct RankedPlans {
    /// The primary (highest-scoring, policy-allowed) plan.
    pub winner: CompatibilityPlan,
    /// Up to 2 runner-up candidates (next-best allowed strategies), not materialized.
    pub runners_up: Vec<PlanCandidate>,
}

/// Like `plan_execution`, but also returns the top-3 ranked candidates.
///
/// `winner` is fully materialized as a `CompatibilityPlan`.
/// `runners_up` are returned as `PlanCandidate` for lightweight inspection (scores, rationale).
pub fn plan_execution_ranked(req: PlanningRequest<'_, '_, '_, '_>) -> Result<RankedPlans, Box<dyn Error>> {
    let gaps = analyze_gaps(req.game, req.target);
    let helper_present = !req.helpers.is_empty();
    let weights = ScoreWeights::default();

    // 1) Score all candidate strategies
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

    // 3) Collect top-3 allowed candidates
    let mut allowed_candidates: Vec<PlanCandidate> = scored
        .iter()
        .filter(|(s, _)| is_allowed(*s, &gaps, req.policy))
        .take(3)
        .map(|(s, scores)| {
            let mut c = PlanCandidate::new(*s);
            c.compensation_map = default_compensation_map_for(*s, &gaps);
            c.scores = *scores;
            c.pipeline = strategy_pipeline(*s, &gaps);
            c.rationale = build_rationale(*s, &gaps, scores);
            c.confidence = derive_confidence(&gaps, scores.total);
            if let Some(mode_id) = req.mode_id {
                apply_mode_split_prefs(&mut c, req.game, mode_id);
            }
            c
        })
        .collect();

    if allowed_candidates.is_empty() {
        let mut nf = PlanCandidate::new(Strategy::NotFeasible);
        nf.rationale = vec!["No feasible strategy found given current gaps and policy.".into()];
        nf.confidence = 0.0;
        allowed_candidates.push(nf);
    }

    // 4) Winner is first candidate — materialize into CompatibilityPlan
    let winner_candidate = allowed_candidates.remove(0);
    let winner = build_compatibility_plan(
        winner_candidate,
        req.game,
        req.target,
        req.helpers,
        &gaps,
        req.mode_id,
    );

    Ok(RankedPlans {
        winner,
        runners_up: allowed_candidates,
    })
}
