use sim_core::ids::{AgentId, ChunkCoord, LinkId};
use sim_core::mobility::lod::{ACTIVITY_HYSTERESIS_TICKS, MobilityActivity};
use sim_core::mobility::{AgentMobilityState, AgentRecord, MobilityWorld, PlanStage};

// One classify pass past the hysteresis window so the new activity has settled.
const SETTLE_TICKS: u32 = ACTIVITY_HYSTERESIS_TICKS as u32 + 10;

#[test]
fn chunk_cycles_through_hot_warm_active() {
    let mut world = MobilityWorld::empty();
    let chunk = ChunkCoord { x: 0, y: 0 };

    // Walk speed 0 so the agent stays in this chunk for the whole test.
    world.set_link_polyline(LinkId("l:0".into()), vec![(5.0, 5.0), (15.0, 15.0)]);
    world.spawn_agent_from_record(AgentRecord {
        id: AgentId("a:1".into()),
        state: AgentMobilityState::Walking {
            link_id: LinkId("l:0".into()),
            progress: 0.0,
        },
        plan: vec![PlanStage::Activity {
            activity_id: "act".into(),
        }],
        plan_cursor: 0,
        walk_speed_per_tick: 0.0,
    });

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
