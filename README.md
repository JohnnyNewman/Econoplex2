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
| sense | Hunger rises; agents buy and eat the best food in their town; health, fatigue and happiness update |
| decide | Each idle adult scores a sample of open jobs and resting: utility = (bias + Wm·mind + Ws·state) · features, softmax with temperature. Jobs fill up: nature, inputs and building slots limit how many can take each one |
| assign | Agents choosing the same job form teams; inputs and nature are reserved; task entities spawn; children join a working parent |
| produce | Teams pool capabilities (element-wise max minus coordination cost); success = sigmoid(k·(effective − difficulty)); quality is the weakest link of step and inputs. The town buys the output and pays wages from its treasury; finished construction becomes a new building |
| learn | Practice with diminishing returns; master-to-apprentice diffusion along the skill direction only |
| socialize | Bounded-confidence alignment on the ideological axes between coworkers and random townspeople |
| body | Norm budget and forgetting on capability; physique trains toward the genetic potential |
| settle | Prices follow stock vs. per-capita targets; buildings are priced by how full they are; food spoils; wealth tax and public spending; caravans trade goods and tools between towns; nature regrows |
| organize | Guild membership and roles from domain proficiency; guild members buy matching tools (on credit if needed) |
| lifecycle | Aging and Gompertz mortality, bequests, partnering by mind affinity, births with genetic inheritance |
| metrics | Yearly: diversity, RCA, economic complexity index, specialization, culture |

Decision features are `[value, skill, effort, novelty, social, need]`. Value is a job's
profit relative to the average job the town offers, so oversupplied trades lose
appeal. Skills are never inherited: children of smiths become smiths only by working
next to their parents. The need feature counts a job's food potential, so milling and
cutting wood for the bakery help against hunger too, not only farming and baking.

### Money

Money is never created or destroyed; `econoplex-sim` prints the total each year and
the `money_is_conserved` test checks it. It moves between agents' wealth and town
treasuries:

| Flow | From → to |
| --- | --- |
| Wages: the town market buys each finished output and pays the team `wage_share` of the value added | treasury → workers |
| Food, eaten at the town price (agents with no money still eat) | eater → treasury |
| Tools, bought by guild members, on credit up to `tool_credit` | member → treasury |
| Caravans: the buying town pays the destination price | buyer treasury → seller treasury |
| Wealth tax on savings above `tax_free_wealth` | agent → treasury |
| Public spending of a treasury above its per-capita reserve | treasury → residents |
| Bequests: partner, else children, else the town | dead agent → heirs |

When a treasury runs dry, wages for that day are cut, so pay follows what people
actually spend. All rates are in the `market` section of `models.ron`.

### Construction

Buildings are products with `slots`: how many people can work in one at a time. When a
building type's slots have been about `build_threshold` full for a month and the goods
it makes sell above base price, the town offers to pay for another one, and a crew
with the Construction skill builds it from planks, wood and (for a smithy) iron. A
town also offers a discounted price for a building type it lacks, which is how a town
can enter a new line of work. Construction recipes live in
`assets/content/40_construction.ron`; one crew builds each building type at a time.

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

With the default scenario (four towns, 1,200 agents), nobody starves in the first year
and the population roughly doubles over 30 years while the money supply stays fixed.
Towns specialize differently from identical rules: Ironhold and Brassmoor, with ore, a
furnace and a smithy, become sword makers with the highest complexity scores.
Greenvale, without a smithy, becomes the breadbasket with the lowest score and buys
its sickles from the smiths. Timberwick builds the most (more mills and bakeries,
and its own smithy). Level-3 longswords stay rare.

Performance on one core: about 1.3 ms per tick at 2,400 agents and 33 ms at 18,500.

## Known limitations of this prototype

- The town treasury is the only buyer of production; there are no private firms,
  and weapons still pile up in town armories.
- Prices are in fixed nominal terms, so a growing population with a fixed money
  supply means lower wealth per person rather than lower prices.
- Buildings never wear out or get demolished.
- Squads and combat do not exist yet; organizations are towns and guilds.
- Agent slots are never reused, so memory grows with births.
- `days_per_year` is 120 so generations are visible in short runs.

See the roadmap document for the next phases.
