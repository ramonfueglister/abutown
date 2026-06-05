//! Static citizen↔market binding: each citizen shops at `home_market` and earns
//! wages at `work_market`. Assigned deterministically at seed from market anchor
//! positions; inherited by newborns; persisted in `AgentRecord`. Market ids are
//! raw `u32` (matching `MarketSpec.id` and the persisted record) so this mobility
//! module carries no dependency on `economy::MarketId`.

use bevy_ecs::prelude::Component;

/// The two markets a citizen is bound to. `home_market` is the shopping
/// destination (realized consumption); `work_market` is the wage commute target.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketBinding {
    pub home_market: u32,
    pub work_market: u32,
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
    ranked.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
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
            (9004u32, (208.0, 48.0)),
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
}
