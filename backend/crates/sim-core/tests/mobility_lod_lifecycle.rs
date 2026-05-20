use sim_core::ids::{AgentId, ChunkCoord, LinkId};
use sim_core::mobility::lod::{ACTIVITY_HYSTERESIS_TICKS, MobilityActivity};
use sim_core::mobility::{AgentMobilityState, AgentRecord, MobilityWorld, PlanStage};

// One classify pass past the hysteresis window so the new activity has settled.
const SETTLE_TICKS: u32 = ACTIVITY_HYSTERESIS_TICKS as u32 + 10;

// Ignored in Phase 8a Task 8: chunk-LOD classification moved out of
// `MobilityWorld` into `CoreSet::LodReclassify` (which operates on chunk
// entities owned by `CorePlugin`). The flow-cell spawn/despawn side-effects
// of promote/demote will be reintroduced as event reactors atop
// `ChunkLodChanged` in a later phase. Re-enable once `MobilityWorld` is
// dissolved (Task 9) and the unified world drives this lifecycle.
#[test]
#[ignore = "Phase 8a Task 8: LOD lifecycle moved to CoreSet::LodReclassify; re-enable after Task 9 dissolves MobilityWorld"]
fn chunk_cycles_through_hot_warm_active() {
    let mut world = MobilityWorld::empty();
    let chunk = ChunkCoord { x: 0, y: 0 };

    // Walk speed 0 so the agent stays in this chunk for the whole test.
    world.set_link_polyline(LinkId("l:0".into()), vec![(5.0, 5.0), (15.0, 15.0)]);
    world.spawn_agent_from_record(AgentRecord::new(
        AgentId("a:1".into()),
        AgentMobilityState::Walking {
            link_id: LinkId("l:0".into()),
            progress: 0.0,
        },
        vec![PlanStage::Activity {
            activity_id: "act".into(),
        }],
        0.0,
    ));

    // Two subscribers → Hot.
    world.apply_subscription_diff(&[chunk, chunk], std::iter::empty());
    world.tick_mobility();
    assert_eq!(
        world.activity_for_chunk(chunk),
        Some(MobilityActivity::Hot),
        "two subscribers should drive chunk Hot on first classify tick",
    );

    // Both unsubscribe → after hysteresis the chunk falls to Warm (population
    // > 0, no subscribers) and the agent is demoted into the FlowCell.
    world.apply_subscription_diff(std::iter::empty(), &[chunk, chunk]);
    for _ in 0..SETTLE_TICKS {
        world.tick_mobility();
    }
    assert_eq!(
        world.activity_for_chunk(chunk),
        Some(MobilityActivity::Warm),
        "chunk should be Warm after cooldown with population but no subscribers",
    );
    let cell = world
        .flow_cell_for_chunk(chunk)
        .expect("flow cell exists after demote");
    assert!(
        cell.population >= 1.0,
        "agent collapsed into flow cell, got {}",
        cell.population
    );

    // One subscriber re-attaches → after hysteresis the chunk promotes to
    // Active and the population is re-spawned as discrete agents.
    //
    // Asleep is not exercised here: with one seeded agent population stays
    // > 0, so the chunk holds at Warm rather than transitioning to Asleep.
    world.apply_subscription_diff(&[chunk], std::iter::empty());
    for _ in 0..SETTLE_TICKS {
        world.tick_mobility();
    }
    assert_eq!(
        world.activity_for_chunk(chunk),
        Some(MobilityActivity::Active),
        "single subscriber after cooldown should promote chunk to Active",
    );
    assert!(
        !world.agents().is_empty(),
        "agent re-promoted from flow cell into the world",
    );
}

/// Mirrors the Phase-6 production flow where agents are loaded from a
/// snapshot (or seeded via `from_network`) directly into chunks that have
/// no subscriber. After the first classify pass these chunks must collapse
/// straight from Asleep into Warm AND demote their population into the
/// FlowCell — otherwise every Advance/Output system pays the full
/// per-entity cost for the rest of the run.
#[test]
#[ignore = "Phase 8a Task 8: demote-into-flow-cell side-effect removed; will return as a ChunkLodChanged reactor in a later phase"]
fn unsubscribed_populated_chunks_demote_on_first_classification() {
    let mut world = MobilityWorld::empty();

    // Seed 50 agents across 50 distinct chunks (one each) — no subscribers.
    for i in 0..50_i32 {
        let link_id = LinkId(format!("l:{i}"));
        // Polyline anchors put the agent at world coord (i*64+5, 5) which
        // maps to chunk (i*2, 0) via chunk_of(_, _, 32).
        let x = i * 64 + 5;
        world.set_link_polyline(link_id.clone(), vec![(x as f32, 5.0), (x as f32, 25.0)]);
        world.spawn_agent_from_record(AgentRecord::new(
            AgentId(format!("a:{i}")),
            AgentMobilityState::Walking {
                link_id,
                progress: 0.0,
            },
            vec![PlanStage::Activity {
                activity_id: "stay".into(),
            }],
            0.0,
        ));
    }
    assert_eq!(
        world.agents().len(),
        50,
        "precondition: 50 agents seeded directly into unsubscribed chunks",
    );

    // One tick is enough: track_chunk_populations + classify push the
    // Asleep→Warm transitions, demote should fire on those.
    world.tick_mobility();

    assert_eq!(
        world.agents().len(),
        0,
        "all unsubscribed populated chunks must demote their agents into FlowCells \
         (this is the regression that makes the 100k bench iterate 101k entities per tick)",
    );
}
