use sim_core::ids::{AgentId, ChunkCoord, LinkId};
use sim_core::mobility::api;
use sim_core::mobility::lod::{ACTIVITY_HYSTERESIS_TICKS, MobilityActivity};
use sim_core::mobility::{AgentMobilityState, AgentRecord, PlanStage};

// One classify pass past the hysteresis window so the new activity has settled.
const SETTLE_TICKS: u32 = ACTIVITY_HYSTERESIS_TICKS as u32 + 10;

#[test]
fn chunk_cycles_through_hot_warm_active() {
    let (mut world, mut schedule) = api::empty_world_and_schedule();
    let chunk = ChunkCoord { x: 0, y: 0 };

    api::set_link_polyline(
        &mut world,
        LinkId("l:0".into()),
        vec![(5.0, 5.0), (15.0, 15.0)],
    );
    api::spawn_agent_from_record(
        &mut world,
        AgentRecord::new(
            AgentId("a:1".into()),
            AgentMobilityState::Walking {
                link_id: LinkId("l:0".into()),
                progress: 0.0,
            },
            vec![PlanStage::Activity {
                activity_id: "act".into(),
            }],
            0.0,
        ),
    );

    // Two subscribers → Hot.
    api::apply_subscription_diff(&mut world, &[chunk, chunk], std::iter::empty());
    api::tick_mobility(&mut world, &mut schedule);
    assert_eq!(
        api::activity_for_chunk(&world, chunk),
        Some(MobilityActivity::Hot),
        "two subscribers should drive chunk Hot on first classify tick",
    );

    // Both unsubscribe → after hysteresis the chunk falls to Warm.
    api::apply_subscription_diff(&mut world, std::iter::empty(), &[chunk, chunk]);
    for _ in 0..SETTLE_TICKS {
        api::tick_mobility(&mut world, &mut schedule);
    }
    assert_eq!(
        api::activity_for_chunk(&world, chunk),
        Some(MobilityActivity::Warm),
        "chunk should be Warm after cooldown with population but no subscribers",
    );
    let cell_pop = api::flow_cell_for_chunk(&world, chunk)
        .expect("flow cell exists after demote")
        .population;
    assert!(
        cell_pop >= 1.0,
        "agent collapsed into flow cell, got {cell_pop}",
    );

    api::apply_subscription_diff(&mut world, &[chunk], std::iter::empty());
    for _ in 0..SETTLE_TICKS {
        api::tick_mobility(&mut world, &mut schedule);
    }
    assert_eq!(
        api::activity_for_chunk(&world, chunk),
        Some(MobilityActivity::Active),
        "single subscriber after cooldown should promote chunk to Active",
    );
    assert!(
        !api::agents(&world).is_empty(),
        "agent re-promoted from flow cell into the world",
    );
}

#[test]
fn unsubscribed_populated_chunks_demote_on_first_classification() {
    let (mut world, mut schedule) = api::empty_world_and_schedule();

    for i in 0..50_i32 {
        let link_id = LinkId(format!("l:{i}"));
        let x = i * 64 + 5;
        api::set_link_polyline(
            &mut world,
            link_id.clone(),
            vec![(x as f32, 5.0), (x as f32, 25.0)],
        );
        api::spawn_agent_from_record(
            &mut world,
            AgentRecord::new(
                AgentId(format!("a:{i}")),
                AgentMobilityState::Walking {
                    link_id,
                    progress: 0.0,
                },
                vec![PlanStage::Activity {
                    activity_id: "stay".into(),
                }],
                0.0,
            ),
        );
    }
    assert_eq!(
        api::agents(&world).len(),
        50,
        "precondition: 50 agents seeded directly into unsubscribed chunks",
    );

    api::tick_mobility(&mut world, &mut schedule);

    assert_eq!(
        api::agents(&world).len(),
        0,
        "all unsubscribed populated chunks must demote their agents into FlowCells \
         (this is the regression that makes the 100k bench iterate 101k entities per tick)",
    );
}
