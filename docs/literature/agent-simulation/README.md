# Agent Simulation Literature

Collected on 2026-05-14 for the Abutown agent mobility foundation.

This folder stores focused primary sources for designing large-scale agent
simulation with vehicles, boarding, ECS/data-oriented runtime structure,
simulation LOD, and multiplayer replication.

## Local Sources

### SUMO

SUMO is the strongest practical reference for person mobility stages:
people walk, ride, stop, access stops, wait for vehicles, board, alight, and
continue through a connected plan.

- Persons: [sources/sumo-persons.html](sources/sumo-persons.html)  
  Source: https://sumo.sourceforge.net/docs/Specification/Persons.html  
  Abutown relevance: use `walk`, `ride`, `stop`, and `access` as the baseline
  mental model for `AgentMobilityState`.

- Public Transport: [sources/sumo-public-transport.html](sources/sumo-public-transport.html)  
  Source: https://eclipse.dev/sumo/docs/Simulation/Public_Transport.html  
  Abutown relevance: stops, dwell time, capacity, boarding delay, and route
  definitions should live in the traffic/transit layer, not inside agents.

- Pedestrians: [sources/sumo-pedestrians.html](sources/sumo-pedestrians.html)  
  Source: https://eclipse.dev/sumo/docs/Simulation/Pedestrians.html  
  Abutown relevance: pedestrian movement can later grow from simple edge
  progress to sidewalks, crossings, and interaction models.

- Simulation Loop: [sources/sumo-simulation-loop.html](sources/sumo-simulation-loop.html)  
  Source: https://eclipse.dev/sumo/docs/Developer/Implementation_Notes/Simulation_Loop.html  
  Abutown relevance: fixed simulation phases should be explicit and budgeted.

- Stop Output: [sources/sumo-stop-output.html](sources/sumo-stop-output.html)  
  Source: https://sumo.dlr.de/docs/Simulation/Output/StopOutput.html  
  Abutown relevance: useful metrics vocabulary for boarding delay, stop delay,
  and station load.

- Public Transport Tutorial: [sources/sumo-public-transport-tutorial.html](sources/sumo-public-transport-tutorial.html)  
  Source: https://sumo.dlr.de/docs/Tutorials/PublicTransport.html  
  Abutown relevance: concrete examples of stops, routes, and passengers.

### MATSim

MATSim is the strongest activity-based planning reference: agents carry day
plans made of activities and legs, while physical movement is simulated by
separate network/transport systems.

- MATSim docs: [sources/matsim-docs.html](sources/matsim-docs.html)  
  Source: https://matsim.org/docs/

- MATSim book, part one: [sources/matsim-book-part-one-latest.pdf](sources/matsim-book-part-one-latest.pdf)  
  Source: https://matsim.org/files/book/partOne-latest.pdf  
  Abutown relevance: separate `AgentPlan` from `AgentMobilityState`.

### ECS And Data-Oriented Runtime

- Bevy ECS: [sources/bevy-ecs-docs.html](sources/bevy-ecs-docs.html)  
  Source: https://docs.rs/bevy/latest/bevy/ecs/index.html  
  Abutown relevance: table storage for stable hot components; sparse-set style
  behavior only where churn matters.

- Flecs docs: [sources/flecs-docs.html](sources/flecs-docs.html)  
  Source: https://www.flecs.dev/flecs/md_docs_2Docs.html

- Flecs systems and pipelines: [sources/flecs-systems.html](sources/flecs-systems.html)  
  Source: https://www.flecs.dev/flecs/md_docs_2Systems.html  
  Abutown relevance: pipeline/schedule design, deterministic ordering, and
  multi-threaded table slicing are useful design references even if we use
  Bevy ECS in Rust.

- Unity Entities archetypes: [sources/unity-entities-archetypes.html](sources/unity-entities-archetypes.html)  
  Source: https://docs.unity.cn/Packages/com.unity.entities%401.0/manual/concepts-archetypes.html  
  Abutown relevance: archetype chunk layout reinforces stable component sets
  and avoiding per-tick archetype churn.

### Replication And Multiplayer

- Unity Netcode overview: [sources/unity-netcode-for-entities.html](sources/unity-netcode-for-entities.html)  
  Source: https://docs.unity.com/en-us/multiplayer/netcode/netcode  
  Abutown relevance: server-authoritative DOTS model for complex large-world
  multiplayer.

- Unity ghost snapshots: [sources/unity-netcode-ghost-snapshots.html](sources/unity-netcode-ghost-snapshots.html)  
  Source: https://docs.unity.cn/Packages/com.unity.netcode%401.5/manual/ghost-snapshots.html  
  Abutown relevance: stream relevant ghost/entity chunks over time instead of
  broadcasting the whole world.

### Unreal Mass

The Epic docs are client-rendered and the local HTML files are mostly shell
documents. Keep these as URL references and use the browser for detailed text.

- Mass Gameplay overview shell: [sources/unreal-mass-gameplay-overview.html](sources/unreal-mass-gameplay-overview.html)  
  Source: https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-mass-gameplay-in-unreal-engine?application_version=5.6

- Mass Entity shell: [sources/unreal-mass-entity.html](sources/unreal-mass-entity.html)  
  Source: https://dev.epicgames.com/documentation/en-us/unreal-engine/mass-entity-in-unreal-engine?application_version=5.6

Abutown relevance: Mass splits data fragments, processors, simulation LOD,
representation LOD, replication, movement, and smart objects. That is the
right shape for a game runtime, even if the source snapshots are link-only.

### Papers

- AI Metropolis, MLSys 2025: [sources/ai-metropolis-mlsys-2025.pdf](sources/ai-metropolis-mlsys-2025.pdf)  
  Source: https://proceedings.mlsys.org/paper_files/paper/2025/file/4f31327e046913c7238d5b671f5d820e-Paper-Conference.pdf  
  Abutown relevance: future cold/cognitive agent scheduling should be
  dependency-aware and not globally lockstepped.

- ScaleSim, arXiv 2026: [sources/scalesim-2601.21473.pdf](sources/scalesim-2601.21473.pdf)  
  Source: https://arxiv.org/pdf/2601.21473  
  Abutown relevance: future LLM/cognitive agents should assume sparse
  activation and predictable invocation windows.

- Dynamic LOD for large-scale urban agents, AAMAS 2011:
  [sources/dynamic-lod-large-scale-agent-urban-simulations-aamas2011.pdf](sources/dynamic-lod-large-scale-agent-urban-simulations-aamas2011.pdf)  
  Source: https://aamas.csc.liv.ac.uk/Proceedings/aamas2011/papers/C5_B67.pdf  
  Abutown relevance: use detailed individual simulation only where it matters;
  aggregate or lazily catch up cold populations.

## Design Takeaways For Abutown

- Agents are people, not vehicles.
- Vehicles are separately simulated traffic/transit entities.
- Riding is a relationship: `AgentLocationRef::InVehicle(vehicle_id, seat)`.
- Agent plans should be activity/leg based.
- Mobility states should be simple and explicit: `AtActivity`, `Walking`,
  `WaitingAtStop`, `Boarding`, `InVehicle`, `Alighting`.
- The first implementation slice should prove walk -> wait -> board -> ride ->
  alight -> walk without full traffic AI.
- Hot agents and vehicles can be ECS entities; warm/cold populations should
  collapse to arrays, aggregates, timers, or durable events.
- Replication should stream relevant chunk/entity deltas, not full-world state.
