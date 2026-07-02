//! Static citizen↔market binding: `assign_binding` deterministically chooses a
//! citizen's `(home_market, work_market)` pair from market anchor positions at
//! seed time. Market ids are raw `u32` (matching `MarketSpec.id`) so this
//! mobility module carries no dependency on `economy::MarketId`.
//! (Birth-inheritance and persistence are wired in later tasks.)

use bevy_ecs::prelude::Component;

/// The two markets a citizen is bound to. `home_market` is the shopping
/// destination (realized consumption); `work_market` is the wage commute target.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketBinding {
    pub home_market: u32,
    pub work_market: u32,
}

/// Collect `(market_id, anchor_position)` for every seeded market whose node is
/// present in the current routing `Graph`, reading the economy `Markets` resource
/// and the `Graph` for each market node's position. Returns an empty vec if the
/// economy is not installed.
///
/// Markets whose `node_id` is out of the live graph's range are skipped: the graph
/// can be rebuilt smaller than the one markets were snapped against (e.g.
/// `apply_into_world` reinstalls a snapshot graph), so a stale market node cannot
/// be located — it simply contributes no binding candidate rather than panicking.
pub fn markets_with_positions(world: &bevy_ecs::world::World) -> Vec<(u32, (f32, f32))> {
    let Some(markets) = world.get_resource::<crate::economy::Markets>() else {
        return Vec::new();
    };
    let Some(graph) = world.get_resource::<crate::routing::Graph>() else {
        return Vec::new();
    };
    markets
        .0
        .iter()
        .filter(|(_, site)| (site.node_id.0 as usize) < graph.node_count())
        .map(|(id, site)| (id.0, graph.node(site.node_id).position))
        .collect()
}

/// Deterministically choose (home_market, work_market) for a citizen at `pos`
/// from `markets` (each `(market_id, market_position)`).
///
/// - `home_market` = the market whose anchor is nearest `pos` (tie-break: lower id).
/// - `work_market` = the nearest market that is NOT `home_market` (tie-break: lower
///   id); if only one market exists, `work_market == home_market`.
///
/// Returns `None` only when `markets` is empty. Pure: no RNG, no wall-clock.
pub fn assign_binding(pos: (f32, f32), markets: &[(u32, (f32, f32))]) -> Option<MarketBinding> {
    fn dist2(a: (f32, f32), b: (f32, f32)) -> f32 {
        let dx = a.0 - b.0;
        let dy = a.1 - b.1;
        dx * dx + dy * dy
    }
    // Sort candidates by (distance, id) deterministically.
    let mut ranked: Vec<(u32, f32)> = markets
        .iter()
        .map(|(id, mp)| (*id, dist2(pos, *mp)))
        .collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let home_market = ranked.first()?.0;
    let work_market = ranked
        .iter()
        .find(|(id, _)| *id != home_market)
        .map(|(id, _)| *id)
        .unwrap_or(home_market);
    Some(MarketBinding {
        home_market,
        work_market,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_is_nearest_work_is_second_nearest() {
        // pos near market 9001; 9002 is the second nearest.
        let markets = vec![
            (9001u32, (2.0f32, 3.0f32)),
            (9002u32, (13.0, 3.0)),
            (9004u32, (72.0, 40.0)),
        ];
        let b = assign_binding((3.0, 3.0), &markets).unwrap();
        assert_eq!(b.home_market, 9001);
        assert_eq!(b.work_market, 9002);
    }

    #[test]
    fn single_market_makes_work_equal_home() {
        let markets = vec![(9001u32, (2.0f32, 3.0f32))];
        let b = assign_binding((100.0, 100.0), &markets).unwrap();
        assert_eq!(b.home_market, 9001);
        assert_eq!(b.work_market, 9001);
    }

    #[test]
    fn empty_markets_is_none() {
        assert!(assign_binding((0.0, 0.0), &[]).is_none());
    }

    #[test]
    fn deterministic_tie_break_by_id() {
        // Two markets equidistant from pos → lower id is home.
        let markets = vec![(9002u32, (0.0f32, 1.0f32)), (9001u32, (0.0, -1.0))];
        let b = assign_binding((0.0, 0.0), &markets).unwrap();
        assert_eq!(b.home_market, 9001, "equal distance → lower id wins");
        assert_eq!(b.work_market, 9002);
    }

    #[test]
    fn deterministic_work_tie_break_by_id() {
        // home=9001 nearest; 9002 and 9003 equidistant from pos → work=9002 (lower id).
        let markets = vec![
            (9001u32, (0.0f32, 0.0f32)),
            (9003u32, (10.0, 0.0)),
            (9002u32, (-10.0, 0.0)),
        ];
        let b = assign_binding((0.0, 5.0), &markets).unwrap();
        assert_eq!(b.home_market, 9001);
        assert_eq!(
            b.work_market, 9002,
            "equal distance for work candidates → lower id"
        );
    }
}
