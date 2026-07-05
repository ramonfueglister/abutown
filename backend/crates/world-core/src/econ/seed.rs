//! Authored economy seed on real Winterthur locations (Task 5,
//! docs/superpowers/plans/2026-07-05-mmorpg-m1-persistent-world.md).
//!
//! `data/winterthur/economy.json` authors markets (as WGS84 lon/lat, projected
//! into local world meters with the SAME anchor transform as the geo bake,
//! `scripts/geo/lib/project.mjs`), the firms that trade on them, and the
//! opening cash. `seed_economy` reconstructs the live economy resources from
//! that file, exactly once: it is IDEMPOTENT (non-empty `Markets` → immediate
//! `Ok`), the PR #86 lesson — a hydrated world must never be double-seeded.
//!
//! Authored CONFIG (`capita_baseline`, `ProducerPolicies`) is re-applied on
//! EVERY call, BEFORE the idempotency guard — the #83 lesson: config is not
//! persisted, so a restart would otherwise silently revert it to defaults.
//!
//! Authoring errors (unknown good, dangling market reference, RAW in a
//! recipe, …) PANIC fail-loud, mirroring the old `markets_layer` seeder's
//! `assert!` doctrine; `Result` is reserved for the money/goods book-keeping.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::world::World;
use serde::Deserialize;

use crate::econ::{
    AccountBook, DemandPool, DemandPools, EconomicActorId, EconomyConfig, EconomyError, GOOD_RAW,
    GoodId, HOUSEHOLD_SECTOR, HouseholdSector, InputPool, InputPools, InventoryBook,
    MarketDistances, MarketGoodKey, MarketGoodState, MarketGoods, MarketId, MarketSite, Markets,
    Money, ProducerPolicies, ProducerPolicy, ProductionPool, ProductionPools, Quantity, RawDeposit,
    RawDeposits, Recipe, SupplyPool, SupplyPools, TRANSPORT_OPERATOR, apportion_cash, euclid_m,
};
use crate::model::SimWorld;

// ── Anchor transform (MUST match scripts/geo/lib/project.mjs) ────────────────
// Local equirectangular projection around the KSW anchor (Brauerstrasse 15).
// +x = east, +z = SOUTH (three.js right-handed ground plane).

/// KSW anchor longitude (degrees) — `ANCHOR.lon` in project.mjs.
pub const ANCHOR_LON: f64 = 8.7285;
/// KSW anchor latitude (degrees) — `ANCHOR.lat` in project.mjs.
pub const ANCHOR_LAT: f64 = 47.5069;
/// Mean earth radius in meters — `R` in project.mjs (haversine reference).
pub const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// WGS84 lon/lat → local world meters, byte-for-byte the project.mjs formula:
/// `x = (lon−lon0)·rad·R·cos(lat0)`, `z = −(lat−lat0)·rad·R`.
pub fn lonlat_to_local_m(lon: f64, lat: f64) -> (f32, f32) {
    let rad = std::f64::consts::PI / 180.0;
    let cos0 = (ANCHOR_LAT * rad).cos();
    let x = (lon - ANCHOR_LON) * rad * EARTH_RADIUS_M * cos0;
    let north = (lat - ANCHOR_LAT) * rad * EARTH_RADIUS_M;
    (x as f32, (-north) as f32)
}

// ── Authored JSON schema (data/winterthur/economy.json) ─────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomySeed {
    pub capita_baseline: i64,
    pub markets: Vec<MarketSpec>,
    /// Good name → wire `GoodId` (must agree with the harvested `GOOD_*` ids).
    pub goods: BTreeMap<String, u16>,
    pub firms: Vec<FirmSpec>,
    /// Actor id (as string) → opening cash; the special key `"household"` is
    /// the mean-field household sector's opening cash.
    pub initial_cash: BTreeMap<String, i64>,
}

impl EconomySeed {
    pub fn from_json(json: &str) -> Result<EconomySeed, serde_json::Error> {
        serde_json::from_str(json)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketSpec {
    pub id: u32,
    pub name: String,
    pub lon: f64,
    pub lat: f64,
}

/// One firm: EITHER an extractor (`raw`: RAW faucet → good, sold at `market`)
/// OR a producer (`recipe`: buys inputs at `market`, sells outputs there).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirmSpec {
    pub actor: u64,
    pub market: u32,
    #[serde(default)]
    pub recipe: Option<RecipeSpec>,
    /// `[good_name, qty_per_interval]`.
    #[serde(default)]
    pub raw: Option<(String, i64)>,
    pub interval_ticks: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeSpec {
    #[serde(rename = "in")]
    pub inputs: Vec<(String, i64)>,
    #[serde(rename = "out")]
    pub outputs: Vec<(String, i64)>,
}

// ── Authored economics constants (not in the JSON; mirror the abutopia layer) ─

/// Household consumer pool bidding for FOOD (at the food-selling market).
pub const HOUSEHOLD_FOOD_POOL: EconomicActorId = EconomicActorId(7_001);
/// Household consumer pool bidding for TOOLS (at the tools-selling market).
pub const HOUSEHOLD_TOOLS_POOL: EconomicActorId = EconomicActorId(7_002);

/// (good name, pool actor) of the household demand side. Each good MUST have a
/// selling firm in the seed (validated) — the pool is placed at that market.
const HOUSEHOLD_GOODS: [(&str, EconomicActorId); 2] = [
    ("food", HOUSEHOLD_FOOD_POOL),
    ("tools", HOUSEHOLD_TOOLS_POOL),
];

const HOUSEHOLD_DESIRED_QTY_PER_TICK: i64 = 10;
const HOUSEHOLD_MAX_PRICE: i64 = 2_000;
const HOUSEHOLD_MPC_BPS: i32 = 8_000;
const HOUSEHOLD_AUTONOMOUS: i64 = 5_000;
const PRODUCER_THETA_BPS: u16 = 8_000;
const PRODUCER_BATCHES_TARGET: u32 = 2;
/// Supply/demand pools fire every econ round; the JSON `interval_ticks`
/// (production cadence) gates only `RawDeposit`/`ProductionPool`.
const POOL_INTERVAL_TICKS: u64 = 1;
/// A market anchored further than this from EVERY baked building is an
/// authoring error (wrong anchor / lon-lat swap) — fail loud at seed.
const MARKET_NEAR_BUILDINGS_M: f32 = 5_000.0;

/// Opening reference price per good (Money ×1000 scale) — a legitimate market
/// opening price (data), set only where currently `<= 0`.
fn opening_price(good: &str) -> Money {
    match good {
        "food" | "tools" => Money(1_000),
        "wood" => Money(380),
        other => panic!("seed: no authored opening price for good {other:?}"),
    }
}

/// Seller reservation price per good (Money ×1000 scale).
fn min_price(good: &str) -> Money {
    match good {
        "food" | "tools" => Money(500),
        "wood" => Money(50),
        other => panic!("seed: no authored min price for good {other:?}"),
    }
}

// ── Seeder ───────────────────────────────────────────────────────────────────

/// Reconstruct the live economy from the authored seed. Idempotent: a world
/// that already has markets (hydrate path) only gets its authored CONFIG
/// re-applied and returns `Ok` untouched otherwise.
pub fn seed_economy(
    world: &mut World,
    seed: &EconomySeed,
    sim: &SimWorld,
) -> Result<(), EconomyError> {
    validate_seed(seed);

    // Authored CONFIG — re-applied on EVERY call, BEFORE the idempotency
    // guard (#83 lesson: neither is persisted; a restart must not revert them).
    world
        .get_resource_or_insert_with(EconomyConfig::default)
        .capita_baseline = seed.capita_baseline;
    {
        let mut policies = world.get_resource_or_insert_with(ProducerPolicies::default);
        for firm in seed.firms.iter().filter(|f| f.recipe.is_some()) {
            policies.0.insert(
                EconomicActorId(firm.actor),
                ProducerPolicy {
                    theta_bps: PRODUCER_THETA_BPS,
                    batches_target: PRODUCER_BATCHES_TARGET,
                },
            );
        }
    }

    // Idempotent STATE bootstrap (PR #86 lesson): once seeded the economy
    // persists — a hydrated world finds non-empty Markets and skips.
    if !world
        .get_resource_or_insert_with(Markets::default)
        .0
        .is_empty()
    {
        return Ok(());
    }

    // ── 1) Markets: lon/lat → local meters (anchor transform), fail-loud if a
    // market lands nowhere near the baked world (authoring error).
    let mut positions: Vec<(MarketId, f32, f32)> = Vec::with_capacity(seed.markets.len());
    {
        let mut markets = world.get_resource_or_insert_with(Markets::default);
        for spec in &seed.markets {
            let (x, z) = lonlat_to_local_m(spec.lon, spec.lat);
            assert!(
                !sim.within_radius(x, z, MARKET_NEAR_BUILDINGS_M).is_empty(),
                "seed: market {} ({}) at ({x:.1}, {z:.1}) has no building within {MARKET_NEAR_BUILDINGS_M} m — wrong anchor or lon/lat swap?",
                spec.id,
                spec.name
            );
            let id = MarketId(spec.id);
            markets.0.insert(
                id,
                MarketSite {
                    id,
                    name: spec.name.clone(),
                    x,
                    z,
                },
            );
            positions.push((id, x, z));
        }
    }

    // ── 2) Market distances: euclid over ALL pairs, both directions baked.
    {
        let mut distances = world.get_resource_or_insert_with(MarketDistances::default);
        for (i, &(ma, ax, az)) in positions.iter().enumerate() {
            for &(mb, bx, bz) in positions.iter().skip(i + 1) {
                let d = euclid_m((ax, az), (bx, bz));
                distances.0.insert((ma, mb), d);
                distances.0.insert((mb, ma), d);
            }
        }
    }

    // Make sure every touched book exists before the per-firm passes.
    world.get_resource_or_insert_with(AccountBook::default);
    world.get_resource_or_insert_with(InventoryBook::default);
    world.get_resource_or_insert_with(MarketGoods::default);
    world.get_resource_or_insert_with(SupplyPools::default);
    world.get_resource_or_insert_with(DemandPools::default);
    world.get_resource_or_insert_with(InputPools::default);
    world.get_resource_or_insert_with(ProductionPools::default);
    world.get_resource_or_insert_with(RawDeposits::default);

    // ── 3) Firms.
    for firm in &seed.firms {
        let actor = EconomicActorId(firm.actor);
        let market = MarketId(firm.market);
        if let Some((good_name, qty)) = &firm.raw {
            // Extractor: RAW faucet + RAW→good recipe + a SupplyPool for the
            // output (the harvested pattern; RAW never reaches a market). The
            // opening RAW stock is a 1-interval buffer, like the old seeder.
            let out_good = good_id(seed, good_name);
            let qty = Quantity(*qty);
            world
                .resource_mut::<InventoryBook>()
                .deposit(actor, GOOD_RAW, qty)?;
            world.resource_mut::<RawDeposits>().0.insert(
                actor,
                RawDeposit {
                    good: GOOD_RAW,
                    qty_per_interval: qty,
                    interval_ticks: firm.interval_ticks,
                    last_regen_tick: None,
                },
            );
            world.resource_mut::<ProductionPools>().0.insert(
                actor,
                ProductionPool {
                    actor,
                    recipe: Recipe {
                        inputs: vec![(GOOD_RAW, qty)],
                        outputs: vec![(out_good, qty)],
                    },
                    interval_ticks: firm.interval_ticks,
                    last_generated_tick: None,
                },
            );
            world.resource_mut::<SupplyPools>().0.insert(
                actor,
                SupplyPool {
                    actor,
                    market,
                    good: out_good,
                    offered_qty_per_tick: qty,
                    min_price: min_price(good_name),
                    interval_ticks: POOL_INTERVAL_TICKS,
                    last_generated_tick: None,
                },
            );
        } else {
            // Producer: Leontief recipe + InputPool (derived input demand at
            // the home market, `max_price` discovered by order generation) +
            // sell-side SupplyPool. NO RawDeposit, NO opening inventory: a
            // producer buys its input on the market.
            let recipe = firm.recipe.as_ref().expect("validated: recipe xor raw");
            let (in_name, in_qty) = &recipe.inputs[0];
            let (out_name, out_qty) = &recipe.outputs[0];
            let in_good = good_id(seed, in_name);
            let out_good = good_id(seed, out_name);
            let in_qty = Quantity(*in_qty);
            let out_qty = Quantity(*out_qty);
            world.resource_mut::<ProductionPools>().0.insert(
                actor,
                ProductionPool {
                    actor,
                    recipe: Recipe {
                        inputs: vec![(in_good, in_qty)],
                        outputs: vec![(out_good, out_qty)],
                    },
                    interval_ticks: firm.interval_ticks,
                    last_generated_tick: None,
                },
            );
            world.resource_mut::<InputPools>().0.insert(
                actor,
                InputPool {
                    actor,
                    market,
                    good: in_good,
                    in_qty,
                    out_qty,
                    out_good,
                    interval_ticks: POOL_INTERVAL_TICKS,
                    last_generated_tick: None,
                    max_price: Money::ZERO,
                },
            );
            world.resource_mut::<SupplyPools>().0.insert(
                actor,
                SupplyPool {
                    actor,
                    market,
                    good: out_good,
                    offered_qty_per_tick: out_qty,
                    min_price: min_price(out_name),
                    interval_ticks: POOL_INTERVAL_TICKS,
                    last_generated_tick: None,
                },
            );
        }
    }

    // ── 4) Household demand pools, one per household good, at the market
    // where that good is SOLD (local auction clears without macro flow).
    let mut pool_weights: BTreeMap<EconomicActorId, i64> = BTreeMap::new();
    for (good_name, pool_actor) in HOUSEHOLD_GOODS {
        let good = good_id(seed, good_name);
        let market = selling_market(seed, good_name);
        world.resource_mut::<DemandPools>().0.insert(
            pool_actor,
            DemandPool {
                actor: pool_actor,
                market,
                good,
                desired_qty_per_tick: Quantity(HOUSEHOLD_DESIRED_QTY_PER_TICK),
                max_price: Money(HOUSEHOLD_MAX_PRICE),
                urgency_bps: 0,
                elasticity_bps: 0,
                interval_ticks: POOL_INTERVAL_TICKS,
                last_generated_tick: None,
                last_consumed_tick: None,
                income_last_tick: Money::ZERO,
                mpc_bps: HOUSEHOLD_MPC_BPS,
                autonomous: Money(HOUSEHOLD_AUTONOMOUS),
            },
        );
        pool_weights.insert(pool_actor, 1);
    }

    // ── 5) Opening cash. The seed-mint is the ONLY permitted non-transfer
    // money creation; the SFC audit baselines AFTER seeding. The "household"
    // entry is deposited into HOUSEHOLD_SECTOR and immediately apportioned out
    // to the consumer pools (largest-remainder, sum-preserving): the sentinel
    // MUST net to zero — run_pay_wages_at_tick fails fast on stranded cash.
    for (key, amount) in &seed.initial_cash {
        let amount = Money(*amount);
        match parse_cash_key(key) {
            CashKey::Firm(actor) => world.resource_mut::<AccountBook>().deposit(actor, amount)?,
            CashKey::Household => {
                let payees: Vec<EconomicActorId> = pool_weights.keys().copied().collect();
                let weights: Vec<i64> = pool_weights.values().copied().collect();
                let splits = apportion_cash(&weights, amount.0);
                let mut accounts = world.resource_mut::<AccountBook>();
                accounts.deposit(HOUSEHOLD_SECTOR, amount)?;
                for (idx, pool_actor) in payees.iter().enumerate() {
                    if splits[idx] > 0 {
                        accounts.transfer(HOUSEHOLD_SECTOR, *pool_actor, Money(splits[idx]))?;
                    }
                }
                // apportion_cash is exactly sum-preserving (Σweights > 0,
                // validated) — stranded sentinel cash would be a seed bug.
                if accounts.account(HOUSEHOLD_SECTOR).available != Money::ZERO {
                    return Err(EconomyError::ConservationViolation);
                }
            }
        }
    }

    // ── 6) SFC household sector: equal weight over every household pool.
    const _: () = assert!(HOUSEHOLD_SECTOR.0 == u64::MAX - 1);
    world.insert_resource(HouseholdSector {
        population: 0,
        pool_weights,
    });

    // ── 7) Opening reference prices for every (market, good) any pool
    // touches: consumer pairs (consumption_update needs a positive ewma),
    // supply pairs, and the producer's input/output pairs (participation
    // bound). Legitimate opening DATA — set only where currently <= 0.
    let mut pairs: BTreeSet<(MarketId, GoodId)> = BTreeSet::new();
    for pool in world.resource::<SupplyPools>().0.values() {
        pairs.insert((pool.market, pool.good));
    }
    for pool in world.resource::<DemandPools>().0.values() {
        pairs.insert((pool.market, pool.good));
    }
    for pool in world.resource::<InputPools>().0.values() {
        pairs.insert((pool.market, pool.good));
        pairs.insert((pool.market, pool.out_good));
    }
    {
        let mut goods = world.resource_mut::<MarketGoods>();
        for (market, good) in pairs {
            let key = MarketGoodKey { market, good };
            let state = goods
                .0
                .entry(key)
                .or_insert_with(|| MarketGoodState::new(key));
            let price = opening_price(good_name(seed, good));
            if state.ewma_reference_price.0 <= 0 {
                state.ewma_reference_price = price;
            }
            if state.last_settlement_price.0 <= 0 {
                state.last_settlement_price = price;
            }
        }
    }

    Ok(())
}

enum CashKey {
    Firm(EconomicActorId),
    Household,
}

fn parse_cash_key(key: &str) -> CashKey {
    if key == "household" {
        return CashKey::Household;
    }
    match key.parse::<u64>() {
        Ok(actor) => CashKey::Firm(EconomicActorId(actor)),
        Err(_) => panic!("seed: initial_cash key {key:?} is neither \"household\" nor an actor id"),
    }
}

/// Good name → `GoodId` from the authored mapping; unknown names fail loud.
fn good_id(seed: &EconomySeed, name: &str) -> GoodId {
    match seed.goods.get(name) {
        Some(id) => GoodId(*id),
        None => panic!("seed: good {name:?} not in the authored goods mapping"),
    }
}

/// Reverse lookup for the opening-price table.
fn good_name(seed: &EconomySeed, good: GoodId) -> &str {
    seed.goods
        .iter()
        .find(|(_, id)| GoodId(**id) == good)
        .map(|(name, _)| name.as_str())
        .unwrap_or_else(|| panic!("seed: no name for {good:?} in the authored goods mapping"))
}

/// The market of the firm that SELLS `good_name` (extractor output or recipe
/// output). Validated upfront: exactly the place to anchor household demand.
fn selling_market(seed: &EconomySeed, good_name: &str) -> MarketId {
    seed.firms
        .iter()
        .find(|f| firm_output(f) == good_name)
        .map(|f| MarketId(f.market))
        .unwrap_or_else(|| panic!("seed: no firm sells household good {good_name:?}"))
}

fn firm_output(firm: &FirmSpec) -> &str {
    match (&firm.raw, &firm.recipe) {
        (Some((name, _)), None) => name,
        (None, Some(recipe)) => &recipe.outputs[0].0,
        _ => panic!(
            "seed: firm {} must have exactly one of raw/recipe",
            firm.actor
        ),
    }
}

/// Fail-loud authoring validation (the old markets_layer `assert!` doctrine).
/// Runs on EVERY call, before both the fresh-seed and the hydrate path.
fn validate_seed(seed: &EconomySeed) {
    assert!(!seed.markets.is_empty(), "seed: no markets authored");
    assert!(
        seed.capita_baseline > 0,
        "seed: capita_baseline must be > 0"
    );
    let mut market_ids = BTreeSet::new();
    for market in &seed.markets {
        assert!(
            market_ids.insert(market.id),
            "seed: duplicate market id {}",
            market.id
        );
        assert!(
            !market.name.is_empty(),
            "seed: market {} has no name",
            market.id
        );
    }
    for (name, id) in &seed.goods {
        assert!(
            GoodId(*id) != GOOD_RAW,
            "seed: good {name:?} maps to GOOD_RAW({}) — RAW is structurally non-tradable",
            GOOD_RAW.0
        );
    }
    let mut actors = BTreeSet::new();
    for firm in &seed.firms {
        let actor = firm.actor;
        assert!(actors.insert(actor), "seed: duplicate firm actor {actor}");
        assert!(
            actor != HOUSEHOLD_SECTOR.0
                && actor != TRANSPORT_OPERATOR.0
                && actor != HOUSEHOLD_FOOD_POOL.0
                && actor != HOUSEHOLD_TOOLS_POOL.0,
            "seed: firm actor {actor} collides with a reserved id"
        );
        assert!(
            market_ids.contains(&firm.market),
            "seed: firm {actor}: market {} not authored",
            firm.market
        );
        assert!(
            firm.interval_ticks >= 1,
            "seed: firm {actor}: interval_ticks must be >= 1"
        );
        match (&firm.raw, &firm.recipe) {
            (Some((good, qty)), None) => {
                good_id(seed, good); // known name (panics otherwise)
                assert!(*qty > 0, "seed: firm {actor}: raw qty must be > 0");
            }
            (None, Some(recipe)) => {
                assert!(
                    recipe.inputs.len() == 1 && recipe.outputs.len() == 1,
                    "seed: firm {actor}: exactly one input and one output good \
                     (single-input Leontief InputPool)"
                );
                for (good, qty) in recipe.inputs.iter().chain(recipe.outputs.iter()) {
                    assert!(
                        good_id(seed, good) != GOOD_RAW,
                        "seed: firm {actor}: RAW in a recipe — extractor faucets only"
                    );
                    assert!(*qty > 0, "seed: firm {actor}: recipe qty must be > 0");
                }
            }
            _ => panic!("seed: firm {actor} must have exactly one of raw/recipe"),
        }
    }
    for (key, amount) in &seed.initial_cash {
        parse_cash_key(key); // panics on malformed keys
        assert!(*amount >= 0, "seed: initial_cash[{key}] must be >= 0");
    }
    for (good, _) in HOUSEHOLD_GOODS {
        assert!(
            seed.firms.iter().any(|f| firm_output(f) == good),
            "seed: household good {good:?} has no selling firm — demand would never clear"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The REAL authored file — parser and data can never diverge.
    const ECONOMY_JSON: &str = include_str!("../../../../../data/winterthur/economy.json");

    /// 3-building fixture from `model/mod.rs` (residential B1, commercial A2,
    /// unknown C3) — all buildings within 5 km of every authored market.
    const FIXTURE: &str = r#"{
      "meta": {"anchor": {"lon": 8.7285, "lat": 47.5069}, "bake_version": 1},
      "buildings": [
        {"id":"{B1}","usage":1,"x":0.0,"z":0.0,"area_m2":200.0,"height_m":9.0,"access_edge":5,"access_offset":2.0},
        {"id":"{A2}","usage":2,"x":100.0,"z":0.0,"area_m2":400.0,"height_m":12.0,"access_edge":7,"access_offset":1.0},
        {"id":"{C3}","usage":0,"x":500.0,"z":500.0,"area_m2":50.0,"height_m":4.0,"access_edge":-1,"access_offset":0.0}
      ]}"#;

    fn seeded_world() -> (World, EconomySeed, SimWorld) {
        let sim = SimWorld::load(FIXTURE).expect("fixture must load");
        let seed = EconomySeed::from_json(ECONOMY_JSON).expect("authored economy.json must parse");
        let mut world = World::new();
        seed_economy(&mut world, &seed, &sim).expect("fresh seed must succeed");
        (world, seed, sim)
    }

    #[test]
    fn seeds_four_markets_and_conserves_authored_cash() {
        let (world, seed, _) = seeded_world();
        assert_eq!(world.resource::<Markets>().0.len(), 4);
        let expected: i64 = seed.initial_cash.values().sum();
        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            Money(expected),
            "total_money must equal the authored initial_cash sum (money minted exactly once)"
        );
        // The wage sentinel must start net-zero: run_pay_wages_at_tick fails
        // fast (ConservationViolation) on any stranded HOUSEHOLD_SECTOR cash.
        assert_eq!(
            world
                .resource::<AccountBook>()
                .account(HOUSEHOLD_SECTOR)
                .available,
            Money::ZERO
        );
    }

    #[test]
    fn second_seed_call_is_a_noop() {
        let (mut world, seed, sim) = seeded_world();
        let total_before = world.resource::<AccountBook>().total_money().unwrap();
        let markets_before = world.resource::<Markets>().0.clone();
        let demand_before = world.resource::<DemandPools>().0.len();
        let supply_before = world.resource::<SupplyPools>().0.len();

        seed_economy(&mut world, &seed, &sim).expect("second call must be Ok");

        assert_eq!(
            world.resource::<AccountBook>().total_money().unwrap(),
            total_before,
            "second seed call must not mint money"
        );
        assert_eq!(world.resource::<Markets>().0, markets_before);
        assert_eq!(world.resource::<DemandPools>().0.len(), demand_before);
        assert_eq!(world.resource::<SupplyPools>().0.len(), supply_before);
    }

    #[test]
    fn market_distances_are_symmetric_and_positive() {
        let (world, _, _) = seeded_world();
        let markets: Vec<MarketId> = world.resource::<Markets>().0.keys().copied().collect();
        let distances = world.resource::<MarketDistances>();
        // 4 markets → 4·3 directed pairs, both directions baked.
        assert_eq!(distances.0.len(), 12);
        for &a in &markets {
            for &b in &markets {
                if a == b {
                    continue;
                }
                let ab = distances.0[&(a, b)];
                assert!(ab > 0, "distinct markets must be > 0 m apart ({a:?},{b:?})");
                assert_eq!(ab, distances.0[&(b, a)], "distance must be symmetric");
            }
        }
    }

    #[test]
    fn firms_map_to_pools_and_projection_matches_anchor_transform() {
        let (world, _, _) = seeded_world();
        // 2 extractors (RAW faucets) + 1 producer.
        assert_eq!(world.resource::<RawDeposits>().0.len(), 2);
        assert_eq!(world.resource::<ProductionPools>().0.len(), 3);
        assert_eq!(world.resource::<SupplyPools>().0.len(), 3);
        // Producer 9001 buys wood: exactly one InputPool + its policy.
        assert_eq!(world.resource::<InputPools>().0.len(), 1);
        assert!(
            world
                .resource::<ProducerPolicies>()
                .0
                .contains_key(&EconomicActorId(9_001))
        );
        // Household demand: one pool per household good, weights positive.
        assert_eq!(world.resource::<DemandPools>().0.len(), 2);
        let household = world.resource::<HouseholdSector>();
        assert_eq!(household.population, 0);
        assert!(household.pool_weights.values().all(|w| *w > 0));
        // Marktgasse (8.7296, 47.4996) relative to the KSW anchor: ~82.6 m
        // east, ~811.7 m south — the project.mjs sign conventions.
        let marktgasse = &world.resource::<Markets>().0[&MarketId(1)];
        assert!((82.0..83.5).contains(&marktgasse.x), "x={}", marktgasse.x);
        assert!((811.0..812.5).contains(&marktgasse.z), "z={}", marktgasse.z);
    }
}
