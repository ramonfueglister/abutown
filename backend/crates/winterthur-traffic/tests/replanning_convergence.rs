//! S4 integration: the day-to-day co-evolution must converge toward a
//! stochastic user (Wardrop) equilibrium. We drive the real replanning core
//! (`winterthur_traffic::replanning`) over a classic two-route network whose
//! travel times follow a BPR volume-delay function, and assert that across
//! days the demand split balances the two routes' travel times — the defining
//! property of user equilibrium (Wardrop, 1952). No kernel here: the mobsim
//! is replaced by the analytic BPR delay so the test is fast and the
//! equilibrium is checkable, exactly how MATSim's replanning is validated on
//! two-route / Braess fixtures (Horni, Nagel & Axhausen, 2016).

use winterthur_traffic::replanning::{
    Plan, PlanMemory, ReplanAction, ReplanParams, ScoreParams, charypar_nagel_score, decide_action,
};

/// Route ids as single-lane "routes" (the replanning core treats a route as an
/// opaque `Vec<u32>` identity; here each route is one id).
const ROUTE_A: u32 = 0;
const ROUTE_B: u32 = 1;

/// BPR travel time (s) for `n` vehicles on a route: `free · (1 + 0.15·(n/cap)^4)`.
fn bpr(free_s: f32, cap: f32, n: usize) -> f32 {
    let x = n as f32 / cap;
    free_s * (1.0 + 0.15 * x * x * x * x)
}

#[test]
fn day_to_day_converges_to_wardrop_split() {
    const N: usize = 400;
    const DAYS: u64 = 120;
    // Route A is faster free-flow, so at equilibrium it carries more load.
    let free_a = 600.0f32;
    let free_b = 720.0f32;
    let cap = 200.0f32;
    let sp = ScoreParams::default();
    // A late-arrival penalty would confound the pure route choice, so set the
    // preferred arrival far in the future — scoring reduces to travel time.
    let pref = 1e9f32;
    // MATSim-default 10% replan share: fewer switchers/day damps the
    // best-response overshoot so the system settles into a tight band around
    // the user equilibrium instead of oscillating widely.
    let rp = ReplanParams {
        replan_share: 0.1,
        reroute_share: 1.0, // route choice only (no time mutation in this fixture)
        // Trip-only scores are small (~1 util), so the MATSim-default θ=2 —
        // tuned for full-day activity scores — barely discriminates near
        // equilibrium. A θ matched to the trip-utility scale makes selection
        // decisive and the split settles tight.
        logit_theta: 12.0,
        ..ReplanParams::default()
    };
    let seed = 0x5A4;

    // Every agent starts knowing only route A (the census plan).
    let mut mem: Vec<PlanMemory> = (0..N).map(|_| PlanMemory::seed(vec![ROUTE_A])).collect();

    let route_of = |m: &PlanMemory| m.selected_plan().unwrap().route[0];

    // A stochastic best-response system oscillates in a band around the
    // Wardrop point, so equilibrium is the TIME AVERAGE over a settled tail —
    // exactly how MATSim reports convergence (averaged over iterations), not a
    // single noisy day. Accumulate the last third of days.
    const TAIL_FROM: u64 = DAYS - 40;
    let (mut sum_ta, mut sum_tb, mut sum_na, mut sum_nb, mut tail_days) =
        (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut day0_gap = 0.0f32;

    for day in 0..DAYS {
        // --- mobsim: count load per route, derive BPR travel times ---------
        let n_a = mem.iter().filter(|m| route_of(m) == ROUTE_A).count();
        let n_b = N - n_a;
        let t_a = bpr(free_a, cap, n_a);
        let t_b = bpr(free_b, cap, n_b);
        if day == 0 {
            day0_gap = (t_a - t_b).abs() / t_a.min(t_b);
        }
        if day >= TAIL_FROM {
            sum_ta += t_a as f64;
            sum_tb += t_b as f64;
            sum_na += n_a as f64;
            sum_nb += n_b as f64;
            tail_days += 1.0;
        }

        // --- scoring: record each executed plan's realized travel time -----
        for m in &mut mem {
            let t = if route_of(m) == ROUTE_A { t_a } else { t_b };
            let score = charypar_nagel_score(&sp, t, 0.0, pref);
            m.score_executed(score, sp.learning_rate);
        }

        // --- between-day replanning ---------------------------------------
        // ReRoute adds the route that was FASTER this day (routing on realized
        // weights). Select picks among known plans by logit.
        let faster = if t_a <= t_b { ROUTE_A } else { ROUTE_B };
        for (a, m) in mem.iter_mut().enumerate() {
            match decide_action(seed, day, a as u64, &rp) {
                ReplanAction::Select => {
                    m.select(seed, day, a as u64, rp.logit_theta);
                }
                ReplanAction::ReRoute => {
                    let idx = m.insert_plan(Plan::new(vec![faster], 0), rp.memory_size);
                    // Execute the freshly added (or matched) plan next day.
                    m.selected = Some(idx);
                }
                ReplanAction::TimeMutate => {
                    // Not exercised (reroute_share = 1.0); select defensively.
                    m.select(seed, day, a as u64, rp.logit_theta);
                }
            }
        }
    }

    let avg_ta = (sum_ta / tail_days) as f32;
    let avg_tb = (sum_tb / tail_days) as f32;
    let avg_na = sum_na / tail_days;
    let avg_nb = sum_nb / tail_days;

    // Both routes used (an interior equilibrium, not a corner solution).
    assert!(
        avg_na > 20.0 && avg_nb > 20.0,
        "degenerate split A={avg_na:.0} B={avg_nb:.0}"
    );

    // Wardrop: used routes carry equal travel time. The tail-averaged gap must
    // be small AND a large improvement over the day-0 all-on-A imbalance.
    // The instantaneous equilibrium gap here is ~2-3%; tail-averaging over the
    // best-response oscillation band, through the convex BPR, inflates it to a
    // few percent. A tail-averaged gap under 7% is unambiguously "settled at
    // user equilibrium" for a stochastic best-response system.
    let rel_gap = (avg_ta - avg_tb).abs() / avg_ta.min(avg_tb);
    assert!(
        rel_gap < 0.07,
        "not at equilibrium: avg_t_a={avg_ta:.0} avg_t_b={avg_tb:.0} gap={:.1}% (avg split {avg_na:.0}/{avg_nb:.0})",
        rel_gap * 100.0
    );
    // Learning must have massively closed the day-0 imbalance (all-on-A is a
    // ~180% gap): require at least a 10× reduction.
    assert!(
        rel_gap < day0_gap * 0.1,
        "learning did not converge: day0 gap {:.1}% → tail gap {:.1}%",
        day0_gap * 100.0,
        rel_gap * 100.0
    );

    // Route A (faster free-flow) carries the larger share at equilibrium.
    assert!(
        avg_na > avg_nb,
        "faster route should carry more load: A={avg_na:.0} B={avg_nb:.0}"
    );
}

/// Determinism: the whole multi-day evolution is reproducible from the seed.
#[test]
fn convergence_is_deterministic() {
    let run = || -> (usize, usize) {
        const N: usize = 100;
        let rp = ReplanParams {
            replan_share: 0.2,
            reroute_share: 1.0,
            ..ReplanParams::default()
        };
        let sp = ScoreParams::default();
        let mut mem: Vec<PlanMemory> = (0..N).map(|_| PlanMemory::seed(vec![ROUTE_A])).collect();
        let route_of = |m: &PlanMemory| m.selected_plan().unwrap().route[0];
        for day in 0..40u64 {
            let n_a = mem.iter().filter(|m| route_of(m) == ROUTE_A).count();
            let t_a = bpr(600.0, 50.0, n_a);
            let t_b = bpr(720.0, 50.0, N - n_a);
            for m in &mut mem {
                let t = if route_of(m) == ROUTE_A { t_a } else { t_b };
                m.score_executed(charypar_nagel_score(&sp, t, 0.0, 1e9), sp.learning_rate);
            }
            let faster = if t_a <= t_b { ROUTE_A } else { ROUTE_B };
            for (a, m) in mem.iter_mut().enumerate() {
                match decide_action(0x99, day, a as u64, &rp) {
                    ReplanAction::ReRoute => {
                        let idx = m.insert_plan(Plan::new(vec![faster], 0), rp.memory_size);
                        m.selected = Some(idx);
                    }
                    _ => {
                        m.select(0x99, day, a as u64, rp.logit_theta);
                    }
                }
            }
        }
        let n_a = mem.iter().filter(|m| route_of(m) == ROUTE_A).count();
        (n_a, N - n_a)
    };
    assert_eq!(
        run(),
        run(),
        "same seed must reproduce the same equilibrium split"
    );
}
