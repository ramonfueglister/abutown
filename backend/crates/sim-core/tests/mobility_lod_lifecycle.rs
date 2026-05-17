use sim_core::ids::{AgentId, ChunkCoord, LinkId};
use sim_core::mobility::lod::MobilityActivity;
use sim_core::mobility::{AgentMobilityState, AgentRecord, MobilityWorld, PlanStage};
use std::collections::HashSet;

/// Hot (2 subscribers) → Warm (0 subscribers, population remains) →
/// Active (1 subscriber re-attaches, population re-promotes to entities).
///
/// "Asleep" is not exercised separately because the test seeds a single
/// agent that is never removed; once the agent is demoted into a FlowCell,
/// population stays > 0, which means the chunk stays Warm rather than
/// Asleep. The Hot → Warm → Active path covers the demote / promote
/// round trip we actually need.
#[test]
fn chunk_cycles_through_hot_warm_active() {
    let mut world = MobilityWorld::empty();
    let chunk = ChunkCoord { x: 0, y: 0 };

    // Static agent in the chunk — walk speed 0 so it doesn't drift to a
    // neighbouring chunk during the cooldown ticks.
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
    let empty: HashSet<ChunkCoord> = HashSet::new();
    let mut one = HashSet::new();
    one.insert(chunk);
    world.update_chunk_subscribers(&empty, &one);
    world.update_chunk_subscribers(&empty, &one);
    world.tick_mobility();
    assert_eq!(
        world.activity_for_chunk(chunk),
        Some(MobilityActivity::Hot),
        "two subscribers should drive chunk Hot on first classify tick",
    );

    // Both unsubscribe → after cooldown (30) + one classify tick the chunk
    // should fall through to Warm (population still > 0, no subscribers).
    world.update_chunk_subscribers(&one, &empty);
    world.update_chunk_subscribers(&one, &empty);
    for _ in 0..40 {
        world.tick_mobility();
    }
    assert_eq!(
        world.activity_for_chunk(chunk),
        Some(MobilityActivity::Warm),
        "chunk should be Warm after cooldown with population but no subscribers",
    );

    // Demote should have collapsed the agent into the flow cell.
    let cell = world
        .flow_cell_for_chunk(chunk)
        .expect("flow cell exists after demote");
    assert!(
        cell.population >= 1.0,
        "agent collapsed into flow cell, got {}",
        cell.population
    );

    // Subscribe one → after cooldown the chunk should be Active.
    world.update_chunk_subscribers(&empty, &one);
    for _ in 0..40 {
        world.tick_mobility();
    }
    assert_eq!(
        world.activity_for_chunk(chunk),
        Some(MobilityActivity::Active),
        "single subscriber after cooldown should promote chunk to Active",
    );

    // And the population should have re-promoted into at least one discrete
    // agent in the world.
    assert!(
        !world.agents().is_empty(),
        "agent re-promoted from flow cell into the world",
    );
}
