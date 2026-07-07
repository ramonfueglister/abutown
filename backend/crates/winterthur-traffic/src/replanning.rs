//! Day-to-day replanning core (S4): the pure, deterministic MATSim
//! co-evolutionary logic — plan scoring, a bounded per-agent plan memory, and
//! plan selection — with no ECS, I/O, or router coupling. The shell drives it
//! once per world day at the world-midnight boundary (see
//! `docs/superpowers/specs/2026-07-07-traffic-s4-replanning-design.md`).
//!
//! # What this module owns
//!
//! Each census-trip agent keeps a [`PlanMemory`]: a bounded choice set of
//! [`Plan`]s (route + departure offset), each carrying an EWMA-blended
//! **score**. Between days a fraction of agents *replan* (add a re-routed or
//! time-mutated plan); the rest *select* among existing plans by a
//! multinomial-logit over scores (MATSim `SelectExpBeta`). Every stochastic
//! choice is a pure function of `traffic_core::u01(seed, world_day, agent ^
//! salt)`, so the whole evolution is thread-independent and snapshot-
//! reproducible — the same determinism discipline as `demand-gen` and the
//! kernel noise.
//!
//! Scoring uses the Charypar-Nagel car-trip utility (v1: realized travel time
//! plus a late-arrival penalty against the census-preferred arrival; no
//! activity chain). Route computation and the actual mobsim live in the shell
//! and the kernel respectively — this module never touches them.
//!
//! # References (APA 7)
//! * Horni, A., Nagel, K., & Axhausen, K. W. (Eds.). (2016). *The multi-agent
//!   transport simulation MATSim*. Ubiquity Press. https://doi.org/10.5334/baw
//! * Charypar, D., & Nagel, K. (2005). Generating complete all-day activity
//!   plans with genetic algorithms. *Transportation, 32*(4), 369-397.
//!   https://doi.org/10.1007/s11116-004-8287-y

use traffic_core::u01;

/// Salt streams for the per-agent draws (third `u01` argument), one per
/// decision so two decisions for the same agent on the same world day never
/// correlate.
const SALT_REPLAN: u64 = 0xA4_0001;
const SALT_ACTION: u64 = 0xA4_0002;
const SALT_SELECT: u64 = 0xA4_0003;
const SALT_TIME_MUT: u64 = 0xA4_0004;

/// Charypar-Nagel scoring weights (utility per hour / per hour late). Utility
/// is negative — a plan that travels longer or arrives later scores lower.
#[derive(Debug, Clone, Copy)]
pub struct ScoreParams {
    /// Marginal utility of travel time (utils per second). Negative.
    pub beta_travel: f32,
    /// Marginal utility of late arrival beyond the preferred time (utils per
    /// second late). Negative and steeper than `beta_travel` — being late is
    /// worse than a slow trip (MATSim default ≈ 3× travel).
    pub beta_late: f32,
    /// EWMA blend weight for a freshly executed score into the running score
    /// (`new = (1-α)·old + α·executed`). MATSim's `learningRate`.
    pub learning_rate: f32,
}

impl Default for ScoreParams {
    /// MATSim-typical car values (utils/h converted to utils/s): −6 utils/h
    /// travel, −18 utils/h late, learning rate 0.3.
    fn default() -> Self {
        ScoreParams {
            beta_travel: -6.0 / 3600.0,
            beta_late: -18.0 / 3600.0,
            learning_rate: 0.3,
        }
    }
}

/// The realized utility of one executed trip: travel-time disutility plus a
/// one-sided late-arrival penalty against the preferred arrival second.
/// Early / on-time arrivals incur no penalty (v1 has no early-arrival bonus).
pub fn charypar_nagel_score(
    p: &ScoreParams,
    travel_time_s: f32,
    arrival_s: f32,
    preferred_arrival_s: f32,
) -> f32 {
    let lateness = (arrival_s - preferred_arrival_s).max(0.0);
    p.beta_travel * travel_time_s + p.beta_late * lateness
}

/// One plan in an agent's choice set: the route (lane-id sequence, as the
/// kernel spawns) and a departure offset (seconds relative to the census
/// departure), plus its running EWMA score. `None` score = never executed.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub route: Vec<u32>,
    pub departure_offset_s: i32,
    pub score: Option<f32>,
}

impl Plan {
    pub fn new(route: Vec<u32>, departure_offset_s: i32) -> Self {
        Plan {
            route,
            departure_offset_s,
            score: None,
        }
    }

    /// Blend a freshly executed score into the running score (EWMA); an
    /// unscored plan takes the executed value directly.
    fn record(&mut self, executed: f32, learning_rate: f32) {
        self.score = Some(match self.score {
            None => executed,
            Some(prev) => (1.0 - learning_rate) * prev + learning_rate * executed,
        });
    }

    /// Route + departure identity (ignores the score) — two plans are the
    /// "same choice" iff both match.
    fn same_choice(&self, route: &[u32], offset: i32) -> bool {
        self.departure_offset_s == offset && self.route == route
    }
}

/// A between-day replanning action for one agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplanAction {
    /// Keep the current plans; select one for execution by logit.
    Select,
    /// Add a freshly re-routed plan (on the previous day's realized weights).
    ReRoute,
    /// Add a departure-time-mutated copy of the currently selected plan.
    TimeMutate,
}

/// Replanning tuning (MATSim strategy shares).
#[derive(Debug, Clone, Copy)]
pub struct ReplanParams {
    /// Fraction of agents that replan (rest re-execute a selected plan).
    pub replan_share: f32,
    /// Among replanners, the fraction doing ReRoute (rest do TimeMutate).
    pub reroute_share: f32,
    /// Logit temperature θ for plan selection (MATSim `brainExpBeta`): higher
    /// → greedier toward the best-scored plan.
    pub logit_theta: f32,
    /// Departure-time mutation range (± seconds) for TimeMutate.
    pub time_mut_range_s: i32,
    /// Max plans kept per agent; the worst-scored is dropped when full.
    pub memory_size: usize,
}

impl Default for ReplanParams {
    fn default() -> Self {
        ReplanParams {
            replan_share: 0.1,
            reroute_share: 0.7,
            logit_theta: 2.0,
            time_mut_range_s: 900,
            memory_size: 5,
        }
    }
}

/// Decide an agent's between-day action, purely from `(seed, world_day,
/// agent)`. Draws the replan gate first, then (if replanning) the strategy.
pub fn decide_action(seed: u64, world_day: u64, agent: u64, p: &ReplanParams) -> ReplanAction {
    if u01(seed, world_day, agent ^ SALT_REPLAN) >= p.replan_share {
        return ReplanAction::Select;
    }
    if u01(seed, world_day, agent ^ SALT_ACTION) < p.reroute_share {
        ReplanAction::ReRoute
    } else {
        ReplanAction::TimeMutate
    }
}

/// A deterministic departure-time mutation offset (± `range`), for TimeMutate.
pub fn time_mutation_offset(seed: u64, world_day: u64, agent: u64, range_s: i32) -> i32 {
    let u = u01(seed, world_day, agent ^ SALT_TIME_MUT); // [0,1)
    (u * (2 * range_s + 1) as f32) as i32 - range_s
}

/// The bounded per-agent plan memory. Snapshot-serialized by the shell.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanMemory {
    pub plans: Vec<Plan>,
    /// Index of the plan chosen for the LAST executed day (for scoring and as
    /// the TimeMutate parent). `None` before the first execution.
    pub selected: Option<usize>,
}

impl PlanMemory {
    /// A memory seeded with the census plan (the initial route, zero offset).
    pub fn seed(route: Vec<u32>) -> Self {
        PlanMemory {
            plans: vec![Plan::new(route, 0)],
            selected: Some(0),
        }
    }

    /// The plan selected for the last executed day.
    pub fn selected_plan(&self) -> Option<&Plan> {
        self.selected.and_then(|i| self.plans.get(i))
    }

    /// Record the executed score against the currently-selected plan (called
    /// after the mobsim day, before between-day replanning).
    pub fn score_executed(&mut self, executed: f32, learning_rate: f32) {
        if let Some(i) = self.selected {
            self.plans[i].record(executed, learning_rate);
        }
    }

    /// Insert a new plan (ReRoute / TimeMutate result). If an identical choice
    /// already exists it is NOT duplicated (its score is kept); otherwise the
    /// plan is added and, if memory is over `memory_size`, the worst-scored
    /// plan is evicted (unscored plans are protected — they deserve a run).
    /// Returns the index the new/existing plan lives at.
    pub fn insert_plan(&mut self, plan: Plan, memory_size: usize) -> usize {
        if let Some(i) = self
            .plans
            .iter()
            .position(|p| p.same_choice(&plan.route, plan.departure_offset_s))
        {
            return i;
        }
        self.plans.push(plan);
        if self.plans.len() > memory_size {
            // Evict the worst SCORED plan (never an unscored one — it has not
            // had its chance). If all are unscored, keep them (shouldn't
            // happen once memory_size ≥ seed+1, but be safe).
            if let Some((worst, _)) = self
                .plans
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.score.map(|s| (i, s)))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            {
                // Preserve the just-added plan's index across the removal.
                let added = self.plans.len() - 1;
                self.plans.remove(worst);
                self.selected = None; // indices shifted; re-select next.
                return if added > worst { added - 1 } else { added };
            }
        }
        self.plans.len() - 1
    }

    /// Select a plan for execution by multinomial logit over scores
    /// (`SelectExpBeta`): `P(i) ∝ exp(θ · score_i)`. Unscored plans are given
    /// the current best score so a fresh plan is explored on its first day
    /// rather than starved. Deterministic in `(seed, world_day, agent)`.
    /// Records the choice in `selected` and returns it.
    pub fn select(&mut self, seed: u64, world_day: u64, agent: u64, theta: f32) -> usize {
        debug_assert!(!self.plans.is_empty(), "cannot select from empty memory");
        let best = self
            .plans
            .iter()
            .filter_map(|p| p.score)
            .fold(f32::NEG_INFINITY, f32::max);
        // Numerically stable softmax: subtract the max exponent.
        let scores: Vec<f32> = self
            .plans
            .iter()
            .map(|p| theta * p.score.unwrap_or(best))
            .collect();
        let max = scores.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let weights: Vec<f32> = scores.iter().map(|s| (s - max).exp()).collect();
        let total: f32 = weights.iter().sum();
        let u = u01(seed, world_day, agent ^ SALT_SELECT) * total;
        let mut acc = 0.0;
        let mut chosen = self.plans.len() - 1;
        for (i, w) in weights.iter().enumerate() {
            acc += w;
            if u < acc {
                chosen = i;
                break;
            }
        }
        self.selected = Some(chosen);
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_penalises_travel_and_lateness_one_sided() {
        let p = ScoreParams::default();
        // On time: only travel disutility.
        let on_time = charypar_nagel_score(&p, 600.0, 1000.0, 1000.0);
        assert!((on_time - p.beta_travel * 600.0).abs() < 1e-6);
        // Early arrival: no bonus, same as on time for equal travel.
        let early = charypar_nagel_score(&p, 600.0, 900.0, 1000.0);
        assert_eq!(on_time, early);
        // Late arrival: strictly worse.
        let late = charypar_nagel_score(&p, 600.0, 1300.0, 1000.0);
        assert!(late < on_time);
    }

    #[test]
    fn ewma_blends_executed_scores() {
        let mut plan = Plan::new(vec![0, 1], 0);
        plan.record(-1.0, 0.3);
        assert_eq!(plan.score, Some(-1.0)); // first exec takes the value
        plan.record(-2.0, 0.3);
        // 0.7*-1 + 0.3*-2 = -1.3
        assert!((plan.score.unwrap() + 1.3).abs() < 1e-6);
    }

    #[test]
    fn decide_action_matches_shares_over_population() {
        let p = ReplanParams::default();
        let n = 100_000u64;
        let (mut replan, mut reroute) = (0u64, 0u64);
        for a in 0..n {
            match decide_action(0xD3, 7, a, &p) {
                ReplanAction::Select => {}
                ReplanAction::ReRoute => {
                    replan += 1;
                    reroute += 1;
                }
                ReplanAction::TimeMutate => replan += 1,
            }
        }
        let replan_frac = replan as f32 / n as f32;
        assert!(
            (replan_frac - p.replan_share).abs() < 0.01,
            "replan {replan_frac}"
        );
        let reroute_frac = reroute as f32 / replan as f32;
        assert!(
            (reroute_frac - p.reroute_share).abs() < 0.02,
            "reroute {reroute_frac}"
        );
    }

    #[test]
    fn insert_dedupes_and_bounds_memory_evicting_worst() {
        let mut m = PlanMemory::seed(vec![0, 1]);
        m.plans[0].score = Some(-5.0);
        // Same choice as the seed: no duplicate.
        let i = m.insert_plan(Plan::new(vec![0, 1], 0), 3);
        assert_eq!(i, 0);
        assert_eq!(m.plans.len(), 1);
        // Add distinct scored plans up to the bound.
        let mut good = Plan::new(vec![0, 2], 0);
        good.score = Some(-1.0);
        m.insert_plan(good, 3);
        let mut mid = Plan::new(vec![0, 3], 0);
        mid.score = Some(-3.0);
        m.insert_plan(mid, 3);
        assert_eq!(m.plans.len(), 3);
        // Adding a 4th over cap evicts the worst SCORED (-5.0, the seed).
        let mut fresh = Plan::new(vec![0, 4], 0);
        fresh.score = Some(-2.0);
        m.insert_plan(fresh, 3);
        assert_eq!(m.plans.len(), 3);
        assert!(!m.plans.iter().any(|p| p.score == Some(-5.0)), "worst kept");
        assert!(m.plans.iter().any(|p| p.route == vec![0, 4]), "new dropped");
    }

    #[test]
    fn select_is_deterministic_and_favours_the_best() {
        // Two plans, one clearly better. Over many (world_day) draws the
        // better plan is chosen far more often, and each draw is reproducible.
        let build = || {
            let mut m = PlanMemory::seed(vec![0, 1]);
            m.plans[0].score = Some(-5.0); // worse
            let mut better = Plan::new(vec![0, 2], 0);
            better.score = Some(-1.0);
            m.plans.push(better);
            m
        };
        let p = ReplanParams::default();
        let mut better_count = 0;
        for d in 0..1000u64 {
            let mut a = build();
            let mut b = build();
            let ia = a.select(0x11, d, 42, p.logit_theta);
            let ib = b.select(0x11, d, 42, p.logit_theta);
            assert_eq!(ia, ib, "same inputs must select the same plan");
            if a.plans[ia].route == vec![0, 2] {
                better_count += 1;
            }
        }
        assert!(
            better_count > 700,
            "logit should favour the better plan, got {better_count}/1000"
        );
    }

    #[test]
    fn unscored_plan_is_explored_not_starved() {
        // A brand-new (unscored) plan alongside a mediocre scored one must
        // still be selectable a meaningful fraction of the time.
        let p = ReplanParams::default();
        let mut chosen_new = 0;
        for d in 0..1000u64 {
            let mut m = PlanMemory::seed(vec![0, 1]);
            m.plans[0].score = Some(-2.0);
            m.plans.push(Plan::new(vec![0, 2], 0)); // unscored
            let i = m.select(0x22, d, 7, p.logit_theta);
            if m.plans[i].route == vec![0, 2] {
                chosen_new += 1;
            }
        }
        assert!(chosen_new > 100, "unscored plan starved: {chosen_new}/1000");
    }
}
