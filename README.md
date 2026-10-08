# Econoplex

An RTS prototype where every unit is an economic agent. Skills, minds, bodies and
needs are vectors, so production, learning, decisions and culture become linear
algebra over thousands of agents. Inspired by agent-based modelling and Hidalgo's
economic complexity: capabilities live in people, spread through apprenticeship,
and towns diversify along the product space.

This is the first prototype: a headless simulation core plus a simple Bevy 2D view.

## Quick start

Requires Rust 1.85 or newer.

```bash
# The game (Bevy 2D view)
cargo run --release -p game

# Headless: 30 years, metrics per year
cargo run --release -p sim_cli -- --years 30

# Bigger world (~12,000 agents), CSV export
cargo run --release -p sim_cli -- --years 20 --scale 10 --csv metrics.csv

# The product space derived from the skill graph
cargo run --release -p sim_cli -- --embedding

# Tests (content validation, models, determinism)
cargo test --release
```

The first build compiles Bevy and takes a while.

### Game controls

| Key | Action |
| --- | --- |
| Space | Pause |
| 1 to 5 | Speed: 1, 4, 12, 30, 60 days per second |
| WASD or arrows | Pan |
| Mouse wheel | Zoom |
| Left click | Inspect an agent (skills, mind, physique vs. genetic potential, state, guild, family) |
| P | Product space overlay: skill graph, brightness = selected agent's (or average) proficiency |
| Esc | Deselect |

Agents are colored by their best skill domain; brightness shows how good they are.
Children are grey and follow a working parent as apprentices.

## Workspace

| Crate | Purpose |
| --- | --- |
| `sim_data` | Content model (skills, products, recipes, nature) loaded from RON, validation, skill embedding |
| `sim_core` | Headless simulation on `bevy_ecs`: agent vector store, entities, tick pipeline, pluggable models, metrics |
| `sim_cli` | `econoplex-sim`: run N years headless, print metrics, export CSV, inspect the embedding |
| `game` | `econoplex`: Bevy app that runs the simulation in `FixedUpdate` and renders it |

All tunable data lives in `assets/`:

- `assets/content/*.ron`: skills, products, recipes, natural resources. Files load
  alphabetically; later files can add or override entries by id (the modding hook).
- `assets/config/models.ron`: every formula constant, plus which model variant to use.
- `assets/config/scenario.ron`: towns, populations, land, buildings, seed.

## How the simulation works

### Agents are vectors

Each agent has an index into `AgentStore`, which keeps every block in contiguous arrays:

| Block | Size | Meaning |
| --- | --- | --- |
| Capability | 32, latent | Skills and know-how |
| Mind | 8, named | ambition, diligence, curiosity, sociability, risk (personality, from genes); tradition, faith, liberty (ideology, social) |
| Physique | 6, named | strength, endurance, agility, dexterity, perception, constitution |
| State | 4, named | hunger, fatigue, happiness, health |
| Genome | fixed | physique potential, temperament, learning aptitude |

### Skills come from the skill graph

Skills form a DAG through prerequisites. Each skill's vector is its prerequisite
closure (itself plus ancestors, weighted by `0.5^depth`), normalized. The dot product
between two skills counts shared prerequisites, so the product space falls out of the
geometry. With more skills than dimensions, NMF compresses it while keeping vectors
nonnegative.

### One tick = one day

| Step | What happens |
| --- | --- |
| sense | Hunger rises; agents eat the best food in their town; health, fatigue and happiness update |
| decide | Each idle adult scores a sample of feasible jobs and resting: utility = (bias + Wm·mind + Ws·state) · features, softmax with temperature |
| assign | Agents choosing the same job form teams; inputs and nature are reserved; task entities spawn; children join a working parent |
| produce | Teams pool capabilities (element-wise max minus coordination cost); success = sigmoid(k·(effective − difficulty)); quality is the weakest link of step and inputs |
| learn | Practice with diminishing returns; master-to-apprentice diffusion along the skill direction only |
| socialize | Bounded-confidence alignment on the ideological axes between coworkers and random townspeople |
| body | Norm budget and forgetting on capability; physique trains toward the genetic potential |
| settle | Prices follow stock vs. per-capita targets; caravans trade between towns; nature regrows |
| organize | Guild membership and roles from domain proficiency; guild members take matching tools |
| lifecycle | Aging and Gompertz mortality, partnering by mind affinity, births with genetic inheritance |
| metrics | Yearly: diversity, RCA, economic complexity index, specialization, culture |

Decision features are `[value, skill, effort, novelty, social, need]`. Value is a job's
profit relative to the average job the town offers, so oversupplied trades lose
appeal. Skills are never inherited: children of smiths become smiths only by working
next to their parents.

### Swapping a formula

Every model is a trait in `crates/sim_core/src/models.rs` (`TeamPooling`,
`SuccessModel`, `QualityModel`, `LearningRule`, `DiffusionRule`, `CapacityRule`,
`SocialInfluence`). To try a new one, implement the trait, add a variant to the matching
enum in `config.rs`, and select it in `models.ron`. For example, change

```ron
team_pooling: ElementwiseMax(coordination_cost: 0.04),
```

to

```ron
team_pooling: LeaderPlusAssistants(assist: 0.2, coordination_cost: 0.04),
```

### Determinism

The tick schedule runs single-threaded in a fixed order, every system draws from its
own RNG stream derived from the seed and tick, and nothing iterates hash maps. Same
seed, same state hash (`econoplex-sim --hash`, and the `same_seed_same_state` test).

## What a 30-year run shows

With the default scenario (four towns, 1,200 agents), towns specialize differently
from identical rules. Brassmoor, with ore, a furnace and a smithy, becomes the
metalworking center with the highest complexity score and most swords. Greenvale,
without a smithy, becomes the breadbasket with the lowest score. Level-3 longswords
appear only rarely, after decades.

Performance on one core: about 1.5 ms per tick at 1,400 agents and 14 ms at 12,000.

## Known limitations of this prototype

- Wages are paid from the output's value, not by a buyer; money is created. Weapons
  and tools accumulate in town armories as the only source of demand.
- Buildings are pre-placed; construction recipes are not modeled yet.
- Squads and combat do not exist yet; organizations are towns and guilds.
- Agent slots are never reused, so memory grows with births.
- The first year has some deaths while labor reallocates to food production.
- `days_per_year` is 120 so generations are visible in short runs.

See the roadmap document for the next phases.
