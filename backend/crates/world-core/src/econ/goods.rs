use crate::econ::GoodId;

pub const GOOD_FOOD: GoodId = GoodId(1);
pub const GOOD_WOOD: GoodId = GoodId(2);
pub const GOOD_IRON: GoodId = GoodId(3);
pub const GOOD_TOOLS: GoodId = GoodId(4);
/// A structurally non-tradable primary resource (the next free `GoodId` after
/// `GOOD_TOOLS`). RAW is NEVER constructed into a `SupplyPool`/`DemandPool`/market
/// seed, so there is no listing path: it can never reach an `OrderBook`/`MarketGoods`.
/// Non-tradability is ENFORCED by absence (no runtime guard). RAW exists only to be
/// deposited by the extractor faucets (`run_regen_at_tick`) and consumed as a recipe
/// input by `run_production_at_tick`.
pub const GOOD_RAW: GoodId = GoodId(5);
