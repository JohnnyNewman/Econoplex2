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

# Change a parameter for one run, and see where the time goes
cargo run --release -p sim_cli -- --years 20 --set military.tribute_share=0.3 --profile

# Parameter sweep: every value on 8 seeds, in parallel, one summary row per setting
cargo run --release -p sim_cli -- --years 30 --seeds 8 \
    --sweep military.tribute_share=0.1,0.2,0.4 --sweep market.free_land=3,6 --csv sweep.csv

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
| sense | Hunger rises; agents buy and eat the best food in their town, leaving the mills' grain reserve alone; health, fatigue and happiness update |
| decide | Each idle adult scores a sample of open jobs and resting: utility = (bias + Wm·mind + Ws·state) · features, softmax with temperature. Jobs fill up: nature, inputs and building slots limit how many can take each one |
| assign | Agents choosing the same job form teams; inputs and nature are reserved; task entities spawn; children join a working parent |
| produce | Teams pool capabilities (element-wise max minus coordination cost); success = sigmoid(k·(effective − difficulty)); quality is the weakest link of step and inputs. The town buys the output and pays wages from its treasury; finished construction becomes a new building |
| learn | Practice with diminishing returns; master-to-apprentice diffusion along the skill direction only |
| socialize | Bounded-confidence alignment on the ideological axes between coworkers and random townspeople |
| body | Norm budget and forgetting on capability; physique trains toward the genetic potential |
| settle | Prices follow stock vs. per-capita targets; buildings are priced by how full they are; food spoils; wealth tax and public spending; caravans trade goods and tools between towns; nature regrows |
| organize | Guild membership and roles from domain proficiency; guild members buy matching tools (on credit if needed) |
| military | Musters (recruit, release, arm with longswords), soldier pay and drill, raids, battles, captives, conquest, tribute and uprisings |
| migrate | Every month some idle adults compare towns; households move where they are better off |
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

### Food

Grain becomes flour in the mills, and flour and wood become bread in the bakeries.
Eaten as bread, a harvest feeds about six times as many people as eaten raw. Fields
and forests regrow at a fixed rate, so every town has a ceiling. Three rules keep
towns from crashing into it:

- **Gluts stop production.** Nobody makes a good once the town holds `glut_factor`
  times its target stock. Mills stop turning edible grain into flour the bakeries
  can't use.
- **Food comes first.** Grain, flour and wood for `food_reserve_days` of the town's
  food need are held back. Hungry people don't eat the mills' grain, and charcoal
  burners and carpenters only get wood beyond what the bakeries need.
- **Births look ahead.** Parents judge the food stock `food_foresight_days` ahead at
  its recent trend. Births slow while the stores are still full but shrinking, so the
  population levels off near what the land can feed instead of overshooting it.

The CLI's `starve` column is the share of residents' days spent starving.

### Land

Towns can raise their ceiling. A cleared field and a woodlot are buildings
(`45_land.ron`) whose product `opens` a nature kind: when the crew finishes, the town
gains a new field or forest site instead of a workplace. The site starts bare and grows
in at the normal regrowth rate. Land is priced like any other building: it is worth
opening while the town's sites of that kind are worn down past `land_threshold` and
what they yield sells above base price. Each town has `free_land` plots (6 by default),
and the CLI lists every town's sites and how many plots are left. Land is short from the
start, so towns use most of their free plots within the first few years.

### Construction

Buildings are products with `slots`: how many people can work in one at a time. When a
building type's slots have been about `build_threshold` full for a month and the goods
it makes sell above base price, the town offers to pay for another one, and a crew
with the Construction skill builds it from planks, wood and (for a smithy) iron. A
town also offers a discounted price for a building type it lacks, which is how a town
can enter a new line of work. Construction recipes live in
`assets/content/40_construction.ron`; one crew builds each building type at a time.

### Squads and raids

Every town keeps a squad of up to `soldier_share` of its adults, as many as its
treasury can pay for a month. Recruits are the most ambitious, risk-loving and
physically strong idle adults. Soldiers stop working, are paid `soldier_pay` a day,
take the best longsword in the armory, and learn the Fighting skill by drilling.

A town whose squad is `raid_margin` times stronger than a neighbor's defense (its
soldiers at home plus a small militia strength per adult) may raid it, more often
when its soldiers are ambitious and risk-loving. The squad marches across the map,
fights on arrival (win chance A^k / (A^k + D^k)), and both sides lose fighters in
proportion to the enemy's strength. A winning raid takes `loot_share` of the
defender's treasury at once and carries the same share of its stores home. It also
takes `captive_share` of the defender's idle adults (at most `captives_per_soldier`
per surviving soldier), who march home with the squad and then live and work in the
captor's town. Fallen fighters die, so their skills die with them; captives take
theirs to the winner. All of this is in the `military` section of `models.ron`.

### Conquest

A winning raid that was `conquest_margin` times stronger than the defense conquers the
defender. Every `tribute_interval` days a conquered town pays its ruler
`tribute_share` of its treasury. It may keep only `vassal_soldier_share` of its adults
under arms, and its ruler's soldiers at home help defend it (`ruler_aid`). Rulers
don't raid their tributaries, and tributaries of the same ruler don't raid each other.
A tributary is free again when:
- it beats its ruler in battle,
- its own soldiers outmatch its share of the ruler's squad (`garrison_strength`, split
  among all the ruler's tributaries), so a ruler with many tributaries or a beaten
  army loses them, or
- by chance, at `independence_rate` a year.

Conquering a town frees the towns it ruled. The CLI shows each town's ruler at year
end, and the CSV adds conquests, uprisings and tribute.

### Migration

Every `interval` days, each idle adult has a `consider` chance to weigh the other
towns. A town's appeal adds up food per head, what the agent's own trade sells for
there compared with the average over towns, how close its people are in outlook, and
a penalty if it was raided this year; the road costs `distance_cost` per 1000 map
units. When another town beats home by `min_gain`, the household moves: the agent,
an idle partner and their young children. They keep their skills, wealth and tools
and join the guild of their new town, so a trade that is better paid elsewhere draws
its practitioners there. The settings are in the `migration` section of `models.ron`,
and the CLI reports arrivals and departures per town each year.

### Trust

Every agent keeps up to eight personal ties, each a trust value between -1 and 1.
Each finished job raises trust between all its coworkers by `cowork_gain`, or by
`failure_factor` of that if the job failed. Chance meetings in town raise trust
between people with similar views and lower it between people far apart. Ties fade
by `fade` a day without contact, and a stronger new tie pushes out the weakest one.
Trust then works back on the economy:

- **Teams** form around the first person to pick a job, who brings the coworkers they
  trust most. A team's proficiency rises by up to `team_bonus` with its cohesion,
  which is the mean trust members place in each other.
- **Teaching:** an apprentice or junior coworker learns up to `1 + teaching_bonus`
  times faster from a master they trust.
- **Squads** fight up to `loyalty` harder at full cohesion. This counts in raids, in
  defense and in a ruler's hold on its tributaries.
- **Migration:** trusted people in a town count toward its appeal, so households tend
  to stay near their friends or follow them.

Ties form mostly at work, so about four in five of them link members of the same
guild, and cultural groups turn into groups of skill. Over 30-year runs on eight
seeds, trust gives about 5% more masters, fewer raids and conquests, and
about a third as many moves as the same rules with every trust effect set to zero. The
CLI shows each town's mean trust per adult and that same-guild share as
`trust/trade`. The settings are in the `trust` section of `models.ron`.

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

### Sweeps

`--set path=value` overrides any value in `models.ron` by its dotted path, including
fields of a selected formula (`success.steepness`). `--sweep path=a,b,c` runs every
value, and several sweeps form a grid; `--seeds N` repeats each setting on N
consecutive seeds. Runs go out to all cores and each comes back as one row:
population, starvation, food, diversity, the ECI range between towns, specialization,
masters, swords, wealth and its Gini between towns, raids, conquests, revolts, the
share of town-years under a ruler, migrants and milliseconds per tick. The table shows
the mean and spread per setting; `--csv` writes every run.

### Determinism

The tick schedule runs single-threaded in a fixed order, every system draws from its
own RNG stream derived from the seed and tick, and nothing iterates hash maps. Same
seed, same state hash (`econoplex-sim --hash`, and the `same_seed_same_state` test).

## What a 30-year run shows

With the default scenario (four towns, 1,200 agents), the population roughly doubles
over 30 years and keeps growing to about 3,500 by year 40, while the money supply stays
fixed. Over 40-year runs on eight seeds, nobody starved after the towns cleared their
free land.
Towns specialize differently from identical rules: Ironhold and Brassmoor, with ore, a
furnace and a smithy, become sword makers with the highest complexity scores.
Greenvale, without a smithy, becomes the breadbasket with the lowest score and buys
its sickles from the smiths. Timberwick builds the most (more mills and bakeries,
and its own smithy). Level-3 longswords stay rare. The sword towns field the
largest squads and raid their neighbors every year or two, mostly successfully, and
carry off a few captives each time. Crushing wins make tributaries, which
usually rise up again within a few years; one town rarely rules all the others. A few percent of each town's people move every
year, more of them toward towns with food to spare.

Performance on one core: about 1.7 ms per tick at 2,000 agents. With `--scale 42`
(50,400 agents at the start, 63,000 after 15 years) a tick takes 40 ms in the first
years and 55 ms on average over 15 years, well inside the 250 ms a 4 Hz game needs.
`--profile` shows the split: choosing jobs takes about a quarter, forming teams,
producing and the yearly life cycle about an eighth each.

## Known limitations of this prototype

- The town treasury is the only buyer of production; there are no private firms,
  and weapons still pile up in town armories.
- Prices are in fixed nominal terms, so a growing population with a fixed money
  supply means lower wealth per person rather than lower prices.
- Buildings never wear out or get demolished.
- War has raids and conquest but no sieges or occupation: a conquered town keeps its
  own people and economy and only pays tribute. A squad fights as one block with no
  tactical movement, and captives become ordinary townspeople at once.
- Migration only looks at the agent's latest trade, not at everything they could do.
- Each town's land is a fixed budget of plots that it uses up early. Opening land does
  not yet follow population growth, and towns never turn forest into fields or the
  other way round.
- Agent slots are never reused, so memory grows with births.
- `days_per_year` is 120 so generations are visible in short runs.

See the roadmap document for the next phases.
