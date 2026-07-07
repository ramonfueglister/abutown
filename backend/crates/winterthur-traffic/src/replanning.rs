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

use serde::{Deserialize, Serialize};
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Stateful day-to-day replanning manager for the shell: owns one
/// [`PlanMemory`] per recurring census trip (keyed by `(day_kind, trip_index)`
/// — the same trip runs every world day, so that pair is the stable agent
/// identity), tracks in-flight vehicles so an arriving trip can be scored by
/// its realized travel time, and runs the between-day step at world midnight.
///
/// Route-choice only in v1 (the dominant equilibrating effect; see the
/// convergence proof) — departure-time mutation is deferred so the spawner's
/// release schedule stays untouched. Deterministic: all draws go through the
/// pure [`decide_action`] / [`PlanMemory::select`] on `(seed, world_day,
/// trip_index)`, and the between-day pass iterates memories in sorted key
/// order.
///
/// This type deliberately does NOT depend on the kernel, the `Router`, or the
/// net: the shell passes a routing closure and a lane→edge map into
/// [`ReplanningState::between_day`], keeping the learning logic unit-testable
/// in isolation (as the convergence test already exercises the algorithm).
#[derive(Debug, Default)]
pub struct ReplanningState {
    /// `(day_kind, trip_index)` → the trip's evolving plan memory.
    memories: std::collections::BTreeMap<(u8, u32), PlanMemory>,
    /// Live vehicles: kernel slot → the trip it carries + its spawn tick, so a
    /// despawn can be scored against the right memory. Removed on despawn.
    in_flight: std::collections::HashMap<u32, InFlight>,
    params: ReplanParams,
    score: ScoreParams,
    seed: u64,
}

#[derive(Debug, Clone, Copy)]
struct InFlight {
    key: (u8, u32),
    spawn_tick: u64,
}

impl ReplanningState {
    pub fn new(seed: u64, params: ReplanParams, score: ScoreParams) -> Self {
        ReplanningState {
            memories: std::collections::BTreeMap::new(),
            in_flight: std::collections::HashMap::new(),
            params,
            score,
            seed,
        }
    }

    /// The route this trip should execute today, if it has a learned plan
    /// selected — the spawner uses it instead of routing fresh. `None` means
    /// "no memory yet": the spawner routes from the census OD and calls
    /// [`note_spawn`](Self::note_spawn) to seed the memory.
    pub fn planned_route(&self, day_kind: u8, trip_index: u32) -> Option<Vec<u32>> {
        self.memories
            .get(&(day_kind, trip_index))
            .and_then(|m| m.selected_plan())
            .map(|p| p.route.clone())
    }

    /// Register a freshly spawned vehicle. Seeds the trip's memory with the
    /// census route on first sight (the day-0 plan), and records the vehicle
    /// as in-flight for later scoring. `census_route` is the route the spawner
    /// actually placed the vehicle on.
    pub fn note_spawn(
        &mut self,
        veh: u32,
        day_kind: u8,
        trip_index: u32,
        spawn_tick: u64,
        census_route: &[u32],
    ) {
        let key = (day_kind, trip_index);
        self.memories
            .entry(key)
            .or_insert_with(|| PlanMemory::seed(census_route.to_vec()));
        self.in_flight.insert(veh, InFlight { key, spawn_tick });
    }

    /// Register a vehicle's arrival (despawn). Scores the trip's currently
    /// selected plan by its realized travel time (`(despawn − spawn)·dt`), so
    /// the next between-day pass can prefer faster plans. A despawn with no
    /// in-flight record (e.g. a non-census citizen car) is ignored.
    pub fn note_despawn(&mut self, veh: u32, despawn_tick: u64, dt: f32) {
        let Some(f) = self.in_flight.remove(&veh) else {
            return;
        };
        let travel_s = despawn_tick.saturating_sub(f.spawn_tick) as f32 * dt;
        // Score on travel time alone in v1 (preferred arrival far in the
        // future ⇒ no late penalty); the module supports the late term when a
        // preferred arrival is wired in.
        let s = charypar_nagel_score(&self.score, travel_s, 0.0, f32::INFINITY);
        if let Some(m) = self.memories.get_mut(&f.key) {
            m.score_executed(s, self.score.learning_rate);
        }
    }

    /// The between-day step, run once at the world-midnight wrap. For every
    /// trip memory, in deterministic key order: decide the replan action and
    /// apply it. ReRoute recomputes the route on the CURRENT (previous day's
    /// realized) edge weights via `reroute(origin_edge, dest_edge)`; the
    /// origin/dest edges are derived from the selected plan's route ends via
    /// `edge_of_lane`. Select re-picks by logit; TimeMutate collapses to Select
    /// in the route-only v1.
    pub fn between_day(
        &mut self,
        world_day: u64,
        edge_of_lane: impl Fn(u32) -> u32,
        mut reroute: impl FnMut(u32, u32) -> Option<Vec<u32>>,
    ) {
        let keys: Vec<(u8, u32)> = self.memories.keys().copied().collect();
        for key in keys {
            let agent = u64::from(key.1) ^ (u64::from(key.0) << 32);
            let action = decide_action(self.seed, world_day, agent, &self.params);
            let mem = self.memories.get_mut(&key).expect("key from own map");
            match action {
                ReplanAction::Select | ReplanAction::TimeMutate => {
                    mem.select(self.seed, world_day, agent, self.params.logit_theta);
                }
                ReplanAction::ReRoute => {
                    // Derive OD from the current plan's route ends.
                    let od = mem.selected_plan().and_then(|p| {
                        let o = *p.route.first()?;
                        let d = *p.route.last()?;
                        Some((edge_of_lane(o), edge_of_lane(d)))
                    });
                    if let Some((oe, de)) = od
                        && let Some(new_route) = reroute(oe, de)
                        && !new_route.is_empty()
                    {
                        let idx = mem.insert_plan(Plan::new(new_route, 0), self.params.memory_size);
                        mem.selected = Some(idx);
                    } else {
                        mem.select(self.seed, world_day, agent, self.params.logit_theta);
                    }
                }
            }
        }
    }

    /// Score and retire every in-flight vehicle no longer alive in the kernel
    /// (arrival), whatever despawn path removed it — kernel end-of-route, the
    /// stranded rescue, or a manual arrival despawn. `is_alive(veh)` is the
    /// kernel's liveness check; `tick` is the current sim tick. Robust to
    /// slot-reuse as long as it runs each tick AFTER all despawns and BEFORE
    /// the next spawn (mirrors `arrivals_system` ordering), so a freed slot is
    /// scored before it can be re-occupied.
    pub fn reap_arrivals(&mut self, tick: u64, dt: f32, is_alive: impl Fn(u32) -> bool) {
        let dead: Vec<u32> = self
            .in_flight
            .keys()
            .copied()
            .filter(|&veh| !is_alive(veh))
            .collect();
        for veh in dead {
            self.note_despawn(veh, tick, dt);
        }
    }

    /// Number of trips with a plan memory (for telemetry / tests).
    pub fn tracked_trips(&self) -> usize {
        self.memories.len()
    }

    /// Number of vehicles currently in flight (for telemetry / tests).
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.len()
    }

    /// Serialise the learned plan memories for the world snapshot, or `None`
    /// when nothing has been learned yet (so the snapshot omits the field
    /// entirely on a fresh world). `in_flight` is deliberately excluded — the
    /// kernel fleet is not persisted, so on resume every trip re-spawns and
    /// re-registers; `params`/`score`/`seed` are authored config, reapplied at
    /// each boot by [`ReplanningState::new`]. Sorted (`BTreeMap` iteration) ⇒
    /// byte-stable JSON, the same discipline as the econ snapshot.
    pub fn to_snapshot_value(&self) -> Option<serde_json::Value> {
        if self.memories.is_empty() {
            return None;
        }
        let sorted: Vec<(&(u8, u32), &PlanMemory)> = self.memories.iter().collect();
        serde_json::to_value(sorted).ok()
    }

    /// Reinstate learned plan memories from a snapshot value (resume). Replaces
    /// the freshly-constructed (empty) memory map; a decode mismatch on a
    /// legacy/corrupt blob is swallowed so the world still boots and agents
    /// simply re-learn — the same benign degradation as before persistence
    /// existed. Returns how many memories were restored (0 on a miss).
    pub fn restore_from_snapshot_value(&mut self, value: &serde_json::Value) -> usize {
        match serde_json::from_value::<Vec<((u8, u32), PlanMemory)>>(value.clone()) {
            Ok(entries) => {
                self.memories = entries.into_iter().collect();
                self.memories.len()
            }
            Err(_) => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kernel slot id as a wire vehicle id (identity here; the state keys on
    /// the raw slot).
    fn veh(slot: u32) -> u32 {
        slot
    }

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
    // ── ReplanningState (shell manager) ────────────────────────────────────

    #[test]
    fn state_seeds_memory_on_first_spawn_and_scores_on_arrival() {
        let mut st = ReplanningState::new(0x5, ReplanParams::default(), ScoreParams::default());
        // No memory yet → spawner must route fresh.
        assert!(st.planned_route(0, 7).is_none());
        st.note_spawn(veh(1), 0, 7, 100, &[3, 4, 5]);
        assert_eq!(st.tracked_trips(), 1);
        assert_eq!(st.in_flight_count(), 1);
        // Next day the same trip has a selected (census) plan to execute.
        assert_eq!(st.planned_route(0, 7), Some(vec![3, 4, 5]));
        // Arrival after 600 ticks → the selected plan gets scored, vehicle
        // leaves the in-flight set.
        st.note_despawn(veh(1), 700, 0.1);
        assert_eq!(st.in_flight_count(), 0);
        let mem = st.memories.get(&(0u8, 7u32)).unwrap();
        assert!(mem.plans[0].score.is_some(), "arrival must score the plan");
        // An unknown despawn (non-census car) is a no-op.
        st.note_despawn(veh(999), 800, 0.1);
    }

    #[test]
    fn state_reroute_adds_and_selects_new_route_on_realized_weights() {
        let rp = ReplanParams {
            replan_share: 1.0,  // force everyone to replan this test
            reroute_share: 1.0, // force ReRoute
            ..ReplanParams::default()
        };
        let mut st = ReplanningState::new(0x9, rp, ScoreParams::default());
        st.note_spawn(veh(1), 0, 1, 0, &[10, 11]); // route ends on lanes 10..11
        // edge_of_lane: lane/10 (so lane 10→edge 1, lane 11→edge 1, dest via last).
        let edge_of = |l: u32| l / 10;
        // reroute returns a DIFFERENT route for the same OD (a faster path today).
        let rr = |_o: u32, _d: u32| Some(vec![10, 99, 11]);
        st.between_day(1, edge_of, rr);
        let mem = st.memories.get(&(0u8, 1u32)).unwrap();
        // The new route was added and selected for execution.
        assert_eq!(st.planned_route(0, 1), Some(vec![10, 99, 11]));
        assert!(
            mem.plans.len() >= 2,
            "rerouted plan must be added to memory"
        );
    }

    #[test]
    fn state_between_day_is_deterministic() {
        let build = || {
            let mut st =
                ReplanningState::new(0xABCD, ReplanParams::default(), ScoreParams::default());
            for i in 0..50u32 {
                st.note_spawn(veh(i), 0, i, 0, &[i * 2, i * 2 + 1]);
                st.note_despawn(veh(i), 100 + i as u64, 0.1);
            }
            st
        };
        let run = |mut st: ReplanningState| -> Vec<Option<Vec<u32>>> {
            let edge_of = |l: u32| l / 2;
            st.between_day(1, edge_of, |_o, _d| Some(vec![0, 1]));
            (0..50u32).map(|i| st.planned_route(0, i)).collect()
        };
        assert_eq!(run(build()), run(build()), "same seed/day → same plans");
    }

    #[test]
    fn snapshot_value_round_trips_plan_memories() {
        // Build a state carrying learned + scored memories.
        let mut src = ReplanningState::new(0x77, ReplanParams::default(), ScoreParams::default());
        for i in 0..8u32 {
            src.note_spawn(veh(i), 0, i, 0, &[i * 3, i * 3 + 1]);
            src.note_despawn(veh(i), 200 + i as u64, 0.1);
        }
        src.note_spawn(veh(50), 1, 3, 0, &[9, 9, 9]); // a second day_kind
        assert_eq!(src.tracked_trips(), 9);

        // Serialise → restore into a fresh state (params/seed re-authored).
        let value = src
            .to_snapshot_value()
            .expect("non-empty memory serialises");
        let mut dst = ReplanningState::new(0x77, ReplanParams::default(), ScoreParams::default());
        let restored = dst.restore_from_snapshot_value(&value);
        assert_eq!(restored, 9, "all memories restored");
        assert_eq!(dst.memories, src.memories, "memories byte-identical");
        // in_flight is NOT carried — a resumed world re-spawns its fleet.
        assert_eq!(dst.in_flight_count(), 0);

        // A fresh (unlearned) state has nothing to persist.
        let empty = ReplanningState::new(0x1, ReplanParams::default(), ScoreParams::default());
        assert!(empty.to_snapshot_value().is_none());

        // A corrupt/legacy blob degrades benignly to "no memories restored".
        let mut victim = ReplanningState::new(0x1, ReplanParams::default(), ScoreParams::default());
        assert_eq!(
            victim.restore_from_snapshot_value(&serde_json::json!({"garbage": true})),
            0,
            "decode miss keeps the empty map, never panics"
        );
        assert_eq!(victim.tracked_trips(), 0);
    }
}
