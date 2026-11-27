# Inventions and knowledge

Design note, agreed in discussion with Nils on 2026-11-26. Steps 1 (content and
validation), 2 (knowledge per person, `knowledge.rs`, `knowledge.enabled`) and 4
(inventing, `knowledge.invention.enabled`) are built and on by default since 2026-11-27.

## Goal

Recipes are not all known from the start. They are discovered by people, passed on by
apprenticeship and schooling, kept secret by guilds, and lost when the last person who
knows them dies. This gives the tech graph a time axis that comes from the agents, not from eras
(eras stay a label). The graph should eventually reach light-fantasy, steampunk-like technology.

## Core idea

Every person has two separate things:

| | What it is | Type | Changes by |
| --- | --- | --- | --- |
| **Knowledge** | which recipe-skills the person knows | bit array, one bit per recipe-skill | invention, apprenticeship, schooling, (later) books |
| **Capability** | how well the person works | the existing capability vector | practice, diffusion, forgetting (as today) |

A person can only work on a recipe-skill they know. How well they do it still follows from
their capability, as it does today. So whoever invents "forged longsword 3" is not good at it yet
and still has to practise.

### Recipe-skills

A recipe-skill is one way of making a good, at one level, for example `forged_longsword_3`.
It corresponds 1-to-1 to one recipe and one node of the tech graph. Recipes to goods are many-to-1:
the same good can be made in different ways, for example

| Good | Ways (recipe-skills) |
| --- | --- |
| iron | bloomery iron (charcoal, small batches), blast furnace iron (coke, bricks, large batches) |
| longsword | forged longsword, pattern-welded longsword (more work, higher quality) |
| bread | hearth bread, oven bread |

Different ways can need different inputs, buildings and skills, so a town picks the way that
fits what it knows and holds, and a new way can make an old one obsolete without removing it.
Levels are successive improvements of one way: each higher level raises the quality you can
reach, saves inputs or time, or does more than one of these. A new way usually builds on an
older way of making the same good plus knowledge from another trade.

Each recipe-skill names the recipe-skills it builds on, in the content:

```ron
(id: "forged_longsword_3", output: "longsword", level: 3,
 requires: ["forged_longsword_2", "steel_1"],
 difficulty: 2.4, invent_difficulty: 3.0, ...)
```

### A graph, not a tree

A node can have several ancestors, and a node can be reached by different routes. So
`requires` is a list of groups: the person needs every group, and within a group any one
member is enough:

```ron
// steel 1 needs iron working, and either a bloomery or a crucible route
(id: "steel_1", output: "steel", level: 1,
 requires: [["iron_smithing_2"], ["bloomery_iron_2", "crucible_melting_1"]], ...)
```

A plain list of single names (`["a", "b"]`) means "a and b", as above. The `requires` links
alone must not loop back on themselves, or the nodes in the loop could never be learned
first. Loops through goods are fine and already exist (iron tools make iron); validation
checks that every node can be reached from the starting knowledge.

This is the existing skill graph (`SkillDef.prereqs`, which already allows several parents)
made fine grained. The embedding in
`sim_data/src/embed.rs` already maps any number of skills into a fixed number of
capability dimensions (closure vectors, NMF when there are more skills than dimensions,
`embedding.dims` in models.ron). So hundreds of recipe-skills do not grow the capability vector.
Each recipe-skill gets its vector `v` from that embedding. Practice along `v` raises the
capability that neighbouring recipe-skills share, so being good at forged longsword 2 helps with
level 3, with pattern welding and with spears.

### Frontier

A person's frontier is every recipe-skill they do not know whose `requires` they meet.
The frontier only changes when the person learns something, so it is cached per person and
recomputed on learning.

```mermaid
flowchart LR
    subgraph known["Known (bits set)"]
        S1[steel_1]
        L1[forged_longsword_1] --> L2[forged_longsword_2]
    end
    subgraph frontier["Frontier (requires met)"]
        L3[forged_longsword_3]
    end
    subgraph hidden["Beyond the frontier"]
        L4[forged_longsword_4]
        P1[pattern_welded_longsword_1]
    end
    L2 --> L3
    S1 --> L3
    L3 --> L4
    L3 --> P1
    P1 --> G((longsword))
    L3 --> G
    L2 --> G
    L1 --> G
```

## Dreaming: the nightly invention check

Every night each adult checks a few frontier items and may invent one of them:

```
p = rate × sigmoid(k · (c · v − invent_difficulty)) × context
```

- `c` is the person's capability vector, `v` the recipe-skill's vector, so `c · v` is how
  close the person's craft is to the new idea. An invention whose `v` spans two trades
  (smithing and carpentry) favours people good at both.
- `rate` is low. Inventions are rare events, about a handful per town per decade. Each one
  goes to the event log with the inventor, town and date.
- `context` comes from the day:
  - **related work:** only frontier items near what the person worked on that day are checked
    (cosine of `v` with today's step skill above a threshold, at most `max_checks` items)
  - **materials at hand:** the inputs exist in the town's stock
  - **need:** the output, or what it improves, is scarce or expensive in the town
  - **time to tinker:** a person who is not exhausted or starving dreams better
  - **contact:** having seen a good made with this recipe-skill (traded in, or carried by
    strangers or enemies) raises the chance, like copying from an example
- Several towns can invent the same thing independently.

## Spreading

- **Apprenticeship:** each day an apprentice works next to a master who knows the recipe-skill,
  the apprentice learns it with probability `apprentice_rate`. Capability diffusion stays as today.
- **Schooling:** a school building with a teacher passes on the teacher's known recipe-skills
  in their domain to many pupils at once, at a lower rate per pupil.
- **Migration and capture:** knowledge moves with people: immigrants, captured craftsmen,
  poached masters.
- **Secrets:** a guild can refuse to teach its recipe-skills to non-members (guild policy lever).
- **Books (later):** writing is itself a recipe-skill. A library holds written recipe-skills and
  teaches them without a living master, and keeps them after their holders die.

## Loss

Knowledge lives in people. When the last person in a town who knows a recipe-skill dies or
leaves, the town can no longer make it. Forgetting of capability stays as today. Knowledge bits
are not forgotten (open: maybe forget a bit after long disuse, to be decided).

## Starting knowledge

At world creation each town gets a knowledge profile from its trades and resources: for each
recipe-skill, the share of people who know it. Every person samples their bits from that
profile, weighted by age and by the trades of their household. Starter recipe-skills are
known by almost everyone in the trade; higher levels by a few masters; everything past the
start set by nobody. The profile is a distribution set in the scenario and content, so worlds
differ by seed.

## What the player sees

The graph starts hidden. A node becomes visible to the player, permanently, the first time any
of the player's people knows it: by inventing it, or by learning it from an immigrant or a
captured smith. Open: whether the frontier of visible nodes is shown as "being worked on".

## Cost

With a few hundred recipe-skills, the bit array is a few `u64` words per person. The frontier
is cached. The nightly pass checks at most `max_checks` related items per working adult, so at
50k agents it is a few hundred thousand dot products per day, well below a tick's cost.

## Build steps

1. Content: recipe-skills with output good, level and `requires` (several ways per good); split today's recipes into level 1
   recipe-skills; validation (every node reachable from the start set)
2. Knowledge bits per person, starting profiles, gating job choice by knowledge (save format) (done)
3. Apprenticeship and migration carry knowledge; loss on death; town knowledge = union of residents (done with step 2)
4. Nightly invention check with context factors; event log (done)
5. Player reveal (simulation side done: `History::revealed`, `ViewSnapshot::revealed`) and the hidden graph in the UI
6. Schools (pupils learn their teacher's recipes: done with step 2) and guild secrets (done: the `secrets TOWN DOMAIN on|off` order)
7. New levels and new items toward the steampunk end; writing and libraries

## As built (steps 2 and 3)

- Recipe-skills with no `requires` start the graph and everyone knows them, so they take
  no bits.
- Founders: every adult knows their family trade (and one way to meet what it requires),
  and each other recipe-skill with `start_share`, or `start_share × foreign_share` when the
  town has no building or land for it. Children start knowing only the roots.
- A regular place needs the recipe-skill known. An apprentice place needs its `requires`
  met (it is on the person's frontier), so the graph is climbed one step at a time.
- Anyone on a team with someone who knows the recipe-skill learns it with
  `1 − (1 − learn_per_day)^days`. Pupils learn each of their teacher's recipe-skills in the
  school's domain at `learn_per_day × school_share`.
- Knowledge is per person, so it moves with migrants and captives and is gone when its
  last holder dies. `knowledge::town_known` counts a town's union.
- Off, nobody holds bits and nothing draws a random number: runs and state hashes are
  exactly as before.

## As built (step 4)

- Nobody knows a recipe-skill of an era after `knowledge.start_era` (3, iron and steel)
  at the start. A recipe's era is the latest of what it makes and what it requires, so
  blast iron (which makes iron, but needs coke) is early industry.
- The check runs after each run of work, for every worker, over the frontier items
  closest to the recipe they worked (skill cosine at least `related`, at most
  `max_checks`), and only for recipe-skills nobody in the town knows.
- The frontier for inventing is shared with the town: the person must know a way to
  meet at least one of the requirements, and the town a way to meet each (a smelter
  who hears of coke from the colliers). Learning it from someone still needs one's own
  requirements met.
- `p` per day = `rate × success(c · v, difficulty + margin) × materials × need × rest`,
  turned into a chance over the run's days. Materials: `no_materials` when the town holds
  too little of an input. Need: the output's price over its base against the town's price
  level, to the power `need_weight`, within 0.5 to 2. Rest: 1 − fatigue. Contact (having
  seen the good) is not in yet.
- At most one idea per worker and run. Each goes to the event log ("invention"). A
  recipe-skill whose inventor dies without teaching anyone is lost and can be invented again.
- With the default rate, in one 30-year run on the scenario's seed, about 20 inventions
  happen across the four towns. They include coke, blast iron and steel, machine parts,
  machine spinning and weaving, and roller milling.

## As built (step 5, simulation side)

- `History::revealed` lists every recipe-skill that anyone in the player's town, or in a
  town it rules, has known, with the day it was first known. It only grows. What the
  founders know shows from the start; every later first is a "discovered" event.
- `ViewSnapshot::revealed` hands a view the newly revealed nodes (all of them on the
  first capture). The CLI prints how many the player has seen, and when the later ones came.
- Without a player town, or with knowledge off, nothing is hidden.

## Open questions

- Do knowledge bits fade after long disuse?
- Is the frontier shown to the player?
- How does a level change a recipe: quality ceiling, input savings, time, or a mix per item?
