# Per-soldier battles

Design note for Phase 6, step 1: a headless battle in which every soldier is
simulated. It is written for review, and none of it is built yet.

## Goal

Today's battle (`sim_core/src/battle/`, squad battles since PR #94) moves
squads as discs and settles each step with dice per squad. Flanks, fronts and
formations exist only as flags: a squad is "flanked" when an enemy it does not
face is in contact, and its front is `min(alive, frontage / spacing)`.

The war redesign note ("Econoplex war redesign", 13 Nov) chose squads as the
unit of war, and set direction A aside: "per-fighter duels stay out unless
squads prove too coarse". Phase 6 brings A back as a **second battle model**
under the same squads:

- Squads stay the unit the player and the AI command, raise, feed and pay. Only
  the battle inside them changes.
- Every soldier has a place, a facing, a morale, a fatigue and their own kit.
  A flank attack, a broken line, a tired army and a wedge come out of those
  positions, not out of a flag.
- The model aims at thousands of soldiers (the plan's target) at the speed of
  the live battle.

The rules of the squad model carry over unchanged in spirit: kit gives reach,
damage and armour; experience counts heavily; morale starts from needs; a
braced spear wall turns a charge; walls narrow the front to the breach. The
existing matchup tests must still pass on the new model.

## Where it sits

The new model lives in `sim_core/src/battle/soldiers/`, next to the squad model.
It takes the same inputs and gives the same outputs:

- **In:** `BattleParams`, `Field`, `Vec<SquadSpec>`, a seed, and orders between
  steps.
- **Out:** `Report`, `ReplayFrame`, `LiveView` and the members read back with
  their health and experience.

So `systems/military` (raids, the planner, live battles), `--battle` and both
clients do not change in step 1.

`models.ron` chooses the model with `battle.model: Squads | Soldiers`. This
follows the standing rule that a component stays swappable, so a seed sweep can
compare the two models. The default stays `Squads` until the budget below has
been measured on real raids.

`Battle` becomes an enum over the two models with the same methods. That is not
a compatibility shim: both models stay real options.

## Data per soldier

Soldiers are stored as plain arrays, one entry per soldier and one array per
field, indexed by soldier. A soldier is never an ECS entity: the battle runs
outside the world, as today.

| Field | Type | Meaning |
| --- | --- | --- |
| `pos` | `Vec2` (m) | position on the field |
| `facing` | `Vec2`, unit | where they look. Kept as a vector, so the hot path needs no `sin` or `cos`. |
| `vel` | `Vec2` (m/s) | last step's velocity, used for charges and impact |
| `health` | `f32` | 1 is unhurt, ≤ 0 is down, ≤ `death_below` is dead (as `Member.health`) |
| `morale` | `f32` 0..1 | the soldier's own nerve. It starts at the squad's morale. |
| `fatigue` | `f32` 0..1 | 0 is rested, 1 is spent. It starts from the person's `fatigue` state. |
| `kit` | `u16` | index into the battle's kit table (below) |
| `squad` | `u16` | the squad they belong to |
| `slot` | `u16` | their place in the squad's formation (file and rank) |
| `skill` | `f32` | effective skill: skill + `experience_weight` · ln(1 + battles) |
| `target` | `u32` or none | the enemy they are fighting |
| `cooldown` | `f32` (s) | time until their next blow or shot |
| `arrows` | `u16` | arrows left |
| `state` | `u8` | Formed, Engaged, Wavering, Routing, Down, Fled |

That is about 72 bytes a soldier, so 4,000 soldiers take under 300 KB.

**Kit.** Each person's kit comes from their own items through
`Kit::from_gear`, as `setup_field_battle` already reads them per person. Kits
that are the same are stored once in a kit table, so a squad with mixed
equipment keeps the difference. A squad then has the best and worst swords, not
an average. The kit table also holds:

- **weight**, which feeds fatigue;
- **mass**, for charges: a horse and rider is about 600 kg, a man about 80 kg.

**Damage types.** The soldier model uses the same split as the squad model,
from the content's `fight` values (PR #178):

- **A blow and a shot have a type.** A weapon's melee `damage_type` and its
  missile `shot_type` are each `Pierce`, `Slash` or `Blunt`. Spears, lances
  and arrows pierce, swords slash, clubs and maces are blunt. Fists are blunt.
- **Armour stops each type by its own share:**
  `armour: (pierce, slash, blunt)`. Layers multiply per type, as in
  `Kit::from_gear`.
- **The damage of a hit** is the sum over types of damage_t × (1 − armour_t),
  as in `Kit::blow` and `Kit::shot`. A charge's extra damage splits in the same
  shares as the blow.

The squad model has to average a squad's weapons into one share per type. The
soldier model needs no average: each soldier's blow carries their own
weapon's type and lands on one target's own armour. Each kit-table entry holds:

- `damage: [pierce, slash, blunt]`, with one non-zero entry for a single weapon;
- `shot_damage` in the same form;
- `armour` per type.

Phase 4 of a substep takes the striker's damage and the target's armour from
the kit table, so per-type damage costs three multiplications and no more
lookups. The tests in the squad model ("mail turns swords better than spears",
"blunt beats slash against plate") also run on the soldier model.

**Fatigue.** This is new, because the squad model has none. It rises each
second by:

- `fatigue_run` × (1 + weight / `carry_kg`) while running;
- `fatigue_fight` while striking;
- `fatigue_walk` while marching.

It falls by `fatigue_rest` while standing out of contact.

Fatigue slows a soldier: their top speed, strike rate and parry are multiplied
by (1 − `fatigue_effect` · fatigue). A spent soldier also loses morale faster.

An army that arrives after a forced march, or one whose soldiers did not sleep,
starts tired. This comes from the store's `fatigue` state, which
`setup_field_battle` passes through a new `Member.fatigue` field.

## The time step

- The soldier model steps in **substeps of 0.25 battle seconds**. A blow takes
  about a second, and a running man covers 0.75 m in a substep, less than the
  1 m spacing between soldiers. With 1 s substeps, runners would pass through
  each other.
- `Battle::step()` still advances one battle second: 4 substeps. `params.dt`
  stays the battle second that callers count in.
- The live battle (`live_battle.rs` in the 2D client, `BATTLE_SPEED = 6`)
  still steps 6 battle seconds per real second: 24 substeps. A paused game steps
  nothing, as today.
- Replays still record a frame every 5 battle seconds (`run_recorded(5.0)`).
  Each frame holds each squad's centre and the soldiers it has left, so the
  clients keep working. A later step can add per-soldier frames for the 3D
  crowd.

One substep runs in six phases. Phases 2 to 4 only read last substep's state
and write into per-soldier buffers, so they can run in parallel. Phases 3 and
4 run in one pass: each soldier's blow is rolled right after their plan.

1. **Grid.** Rebuild the spatial grid (below).
2. **Squad controllers.** Each squad's controller sets its formation, anchor and
   facing from its order, and gives every soldier their slot.
3. **Soldier controllers.** Each soldier picks where to step (their slot, an
   enemy, or away from the enemy) and whom to strike.
4. **Blows and shots.** Each strike rolls to hit and adds its damage to the
   target's incoming buffer. The roll uses the striker's own random stream.
5. **Apply.** In soldier order: damage, movement with collision, morale,
   fatigue and state.
6. **Squads.** Recount each squad's soldiers and morale, and give events to the
   `Report` (contact, charge, wavering, routing, out of arrows, fled).

## The spatial grid

A uniform grid of **4 m cells** covers the field: 600 × 400 m is 150 × 100
cells. A 3 m spear reaches no further than the neighbouring cells, so a melee
query reads at most 3 × 3 cells.

The grid is rebuilt every substep by a counting sort:

- count the soldiers in each cell;
- turn the counts into prefix sums;
- write the soldier indices into one array.

This costs O(n), needs no allocation after the first substep, and visits each
cell's soldiers in index order, so the result never depends on hashing or
thread timing.

The grid answers three queries:

- **Neighbours within 1 m**, for collision: soldiers push apart, and friends
  give way to the front rank.
- **The nearest enemy within reach**, for melee. Each cell is split by side,
  with the enemy side stored first.
- **Who stands where an arrow lands**, for shots. A shot aims at a point with
  scatter that grows with range and falls with skill, then hits whoever is in
  that cell. A shot into a melee can then hit friends. That replaces today's
  "never shoot into a melee" rule. It can be switched off with
  `friendly_fire: false`.

Walls are a line of blocked cells with a gap of `breach_width`. Attackers can
only reach the defenders through the gap. So the breach narrows the front
because the attackers cannot fit through it, not because a front count is
capped.

## Controllers and formations

Command has four levels. Each level only sets goals for the level below it, and
the soldier level is where blows land.

```mermaid
flowchart TD
    P["Player or battle AI<br/>(one commander per side)"] -->|"order per squad:<br/>Auto, Hold, Attack(j), Shoot(j), MoveTo, Withdraw"| S["Squad controller<br/>formation, anchor, facing,<br/>slot per soldier"]
    S -->|"slot and stance"| U["Soldier controller<br/>step to slot or enemy,<br/>pick a target, strike or shoot"]
    U -->|"blows, shots, impact"| B["Blows and shots<br/>to-hit, damage, armour"]
    B -->|"losses, flank hits"| M["Morale and fatigue<br/>soldier, then squad"]
    M -->|"waver or rout"| U
    M -->|"squad morale and state"| S
    S -->|"report, events, live view"| P
```

**Commander, one per side.** This is the player in a live battle, or the auto
commander, which is today's `auto_goal` judgment lifted to the side. Orders are
`Order`, a list per squad carried out in turn, shaped for RTS control (step 5,
"Battle controls"). Going round a squad's side and setting a front are moves
(`MoveTo` with a facing and a width), not orders of their own. A side's
doctrine sets how the auto commander goes about it.

**Squad controller.** It turns the order into an **anchor** (the centre of the
front rank), a **facing** and a **formation**. Each soldier's slot is then:

> anchor + rotate(facing) · (file · spacing, −rank · spacing)

| Formation | Shape | Who uses it on Auto |
| --- | --- | --- |
| Line | `frontage` wide, as many ranks as it takes | infantry |
| Column | 4 wide | marching to a goal far away |
| Wedge | a triangle, point at the front | cavalry charging |
| Braced | Line, with the front ranks kneeling and facing the threat | spears under `Hold`, or when riders come within 200 m |
| Loose | Line at 3 m spacing | archers and skirmishers, or any squad in loose order |

When soldiers fall, the controller fills the front from the rear rank: slots are
reassigned by a stable sort on rank, then on file. So a squad gets thinner and
shorter, but no gap stays open in its front unless the whole squad is gone.

The squad controller also judges as today: archers stand and shoot in range,
riders steer clear of a spear wall, a routing squad runs for its edge of the
field.

**Soldier controller.** This is a small state machine per soldier:

- **Formed.** Steer to the slot.
- **Engaged.** An enemy is within reach, or within 2 m of the slot. Turn to them
  and strike when the cooldown allows. Hold the slot within 2 m unless the
  enemy steps back.
- **Wavering.** Fight at the morale factor, as today.
- **Routing.** Turn and run from the nearest enemy, toward their own edge of the
  field. A routing soldier does not parry.
- **Down.** Lie where they fell. The squad keeps their health for
  `settle_field_battle`.

**Where flanks come from.** A blow is a flank blow when it comes from more than
90° off the target's facing, and a rear blow beyond 135°:

- a flank blow meets no shield and no parry;
- a rear blow also does more damage;
- each blow from the side or rear drains the target's morale by `flank_drain`.

There is no "flanked" flag. A squad hit in the side loses its outer files and
its nerve from that side inward.

**Charges.** On first contact, the impact comes from the charger's mass and
closing speed. A braced spear in front of the charger strikes first, with that
speed: today's rule, but per soldier, and only where a spear point actually
meets the horse. So a charge that hits a braced wall in the flank goes through.

**Morale.** A soldier's morale falls when:

- they are hit, or hit from the side;
- friends within 5 m fall or rout;
- their fatigue rises.

It rises again when they stand quiet next to formed friends, or within 15 m of
the squad's leader (its most experienced member).

The squad's morale is the mean of its soldiers' morale. A squad routs when it
falls below `rout_at`, as today. Single soldiers can also break earlier, so a
squad frays before it breaks.

## Randomness

- Every random draw comes from a counter-based stream: SplitMix64, as
  `AgentRng` already is. It is seeded by battle seed, substep, soldier index
  and purpose (to-hit, damage, scatter, where they step). A ChaCha stream is no
  longer shared across the battle.
- So a draw depends only on who draws it and when, never on the order soldiers
  are processed in or on threads. Phases 2 to 4 can run on rayon and give the
  same battle as a single thread. A test checks this.
- Damage is summed into per-soldier buffers and applied in soldier order in
  phase 5. Two blows on one soldier in the same substep give the same result in
  any order.
- Floats stay deterministic on one binary and one platform. The hot path uses
  only `+ − × ÷`, `sqrt`, `min` and `max`. The to-hit curve uses `exp`, as
  today, which can differ between platforms in the last bit. The state hash is
  compared on the same platform, as today.
- Same seed and same orders give the same battle. Tests run many seeds, never
  one lucky seed.

## Performance budget

**Today's squad model, measured.** These are release builds of
`econoplex-sim --battle`, 50 seeds each, wall time per battle including start-up:

| Line-up | Soldiers | Battle lasts | Per battle | Per battle second |
| --- | --- | --- | --- | --- |
| `swordsmen*30 vs spearmen*30` | 60 | 4 min | 0.2 ms | ~1 µs |
| 3 mixed squads of 40 a side | 240 | 6 min | 0.7 ms | ~2 µs |
| 5 squads of 40 swordsmen a side | 400 | 4 min | 1.5 ms | ~6 µs |
| `swordsmen*400 vs swordsmen*400` | 800 | 8 min | 3.8 ms | ~8 µs |
| `swordsmen*2000 vs swordsmen*2000` | 4,000 | 27 min | 77 ms | ~48 µs |

The squad model's cost grows with the hits, because `hit()` collects the
standing members for each one. That is why 4,000 soldiers cost 48 µs a second,
not 8.

The table also shows something the soldier model should fix. Equal single
squads of 400 or 2,000 are not even: the defenders win 48 and 49 of 50,
although both sides advance on `Auto`. The cause is not known yet. Five squads
of 40 a side split 23 to 27.

**Budget for the soldier model.** It must hold these on one core, with a
4,000-soldier battle (2,000 a side, about 50 squads of 40) as the yardstick:

| Use | Battle time | Budget | Per soldier per substep |
| --- | --- | --- | --- |
| Live battle, 6 battle s per real s | 1 battle s | **≤ 1 ms** (6 ms of every real second) | ≤ 60 ns |
| `--battle`, one auto-resolved battle | about 30 min | ≤ 2 s | same |
| `--battle --seeds 50` | 50 × 30 min | ≤ 2 min single-threaded, ≤ 20 s on 8 threads | same |

So the soldier model may cost about 20 times the squad model at 4,000 soldiers.
It must not cost more than 1 ms per battle second.

**What stays on the squad model.** The war planner's forecasts (`FORECASTS =
3` per possible raid, every muster) and the economy's auto-resolved raids stay
on squads, because they are cheap. A forecast is a guess, so it may use the
cheaper model. Whether auto-resolve moves to soldiers waits for a 40-year
comparison of time per tick and of outcomes.

**How it is measured.** `econoplex-sim --battle L --profile` prints:

- soldiers;
- substeps;
- ms per battle second, for each phase (3 and 4 together, as they run in one
  pass);
- the squad model's figure for the same line-up.

A test in `sim_core/tests/battle_budget.rs` checks the 4,000-soldier budget,
the best of three runs on one core. It runs with `cargo test --release`; debug
builds skip it. It is an integration test so that it times the library as it
ships, not the build of it that carries the unit tests.

## `--battle`

- The line-up syntax stays as it is. `--battle-model squads|soldiers`
  overrides `models.ron`.
- `--compare` runs both models on the same seeds and prints both win rates.
  That is the tool for checking that the soldier model keeps the squad model's
  matchups.
- Two new squad suffixes:
  - `%F`, starting fatigue 0..1 (`swordsmen*100%0.8` is a tired squad);
  - `#flank` or `#rear`, which deploys the squad on the enemy's side or behind
    them, already facing in.
- `--profile` prints the timings above. `--seeds N` prints win rates, as today.

Example:

```
econoplex-sim --battle "swordsmen*100,swordsmen*100#flank vs swordsmen*100,swordsmen*100" \
              --battle-model soldiers --seeds 50 --compare
```

## Tests

These are new, in `battle/soldiers/tests.rs`. Each runs many seeds and checks a
rate with room for chance:

- **Equal armies win about half the time.**
  - `swordsmen*200 vs swordsmen*200`, both sides on `Auto`, deployed as mirror
    images: over 100 seeds, side 0 wins 35 to 65.
  - With the sides swapped, the rate stays within 10 of that.
  - The same holds at 2,000 a side, which the squad model fails today.
- **A flank beats a frontal attack.** Two squads of 100 swordsmen a side.
  - Frontal: side 0's second squad attacks beside its first.
  - Flank: it starts with `#flank` and attacks into the enemy's side.
  - Over 50 seeds, side 0 wins at least 40 battles with the flank and at
    least 15 more than frontally on the same seeds, and the enemy loses more
    soldiers. (The frontal attack is a mirror, so it wins about 25 of 50; a
    margin of 25 would ask the flank to win every battle.)
- **Rested beats exhausted.** `swordsmen*100 vs swordsmen*100%0.8`: the rested
  side wins at least 40 of 50. At fatigue 0.4 it still wins more than half.
- **The squad model's matchups still hold on the soldier model:**
  - a spear wall stops a charge;
  - cavalry rides down archers;
  - arrows break a levy;
  - swords get inside spears;
  - veterans beat more recruits;
  - walls hold the breach;
  - hungry squads break first;
  - routed squads leave the field;
  - orders change the battle.

  These are today's tests in `battle/tests.rs`, run on both models.
- **Determinism:**
  - same seed, same battle;
  - the same battle on 1 thread and on 8;
  - stepping 1 s at a time gives the same battle as `run()`.
- **Formations:** a squad that loses its front rank refills it from the rear. A
  braced line facing riders turns to face them.
- **Budget:** 4,000 soldiers stay within 1 ms per battle second (release
  builds only).

## Build order

1. **This note.** Then the model on its own: data, grid, controllers, blows,
   morale and fatigue, behind `battle.model`. Add `--battle-model`, `--compare`
   and `--profile`, and the tests above. No client changes.
2. Tune until `--compare` keeps the squad model's matchups and the equal-armies
   test holds at every size.
3. Live battles on the soldier model, in both clients: the panels stay, and the
   squads are drawn from their soldiers' positions.
4. Per-soldier replay frames, for the 3D crowd (`crowd.rs` can take one
   record per soldier). Done: see "Steps 4 and 5" below.
5. Going round a flank and setting a front from the battle panels. Done, as
   an order API for RTS control (see "Battle controls").
6. A 40-year comparison of auto-resolve on soldiers against squads. Switch the
   default only if it is affordable and the outcomes make sense.

## Where steps 1 and 2 stand

The model is in `battle/soldiers/`, behind `battle.model` (default `Squads`).
Tuning added a few rules that the note above did not name:

- A squad starts facing the enemy army as a whole, not the nearest squad, so
  `#flank` meets an enemy that has to wheel.
- A soldier turns at `soldier_turn` radians per second. A blow from a side
  they have not turned to yet is a flank blow.
- Any steady soldier who arrives at a run charges, not only one who is still
  formed. Before this, only the point of a wedge charged.
- Seeing a foe break gives a soldier heart (`foe_breaks`). Without it, two
  squads often broke together, and a draw goes to the defenders: mirror images
  split 37 to 63.
- Two counts per squad. Cohesion counts only the fallen, so a rout does not
  feed on itself. Refilling the ranks counts everyone who left the line.
- Defenders on a wall strike over twice the breach width, and a man climbing
  through it cannot parry.

`--compare`, 50 seeds, side 0's wins (squads / soldiers):

| Line-up | Squads | Soldiers |
| --- | --- | --- |
| `cavalry*20 vs spearmen*30` | 0 | 1 |
| `cavalry*20 vs archers*30` | 50 | 47 |
| `levy*40 vs archers*30` | 0 | 0 |
| `swordsmen*30 vs spearmen*30` | 50 | 50 |
| `swordsmen*25+20 vs swordsmen*30` | 50 | 50 |
| `swordsmen*45 vs walls spearmen*30` | 0 | 0 |
| `swordsmen*30!0.3 vs swordsmen*30!0.9` | 0 | 0 |
| `swordsmen*100 vs swordsmen*100%0.8` | 22 | 50 |
| two squads a side, frontal | 29 | 24 |
| the same with `#flank` | 19 | 48 |
| `swordsmen*200 vs swordsmen*200` | 25 | 24 |
| `swordsmen*2000 vs swordsmen*2000` | 26 | 27 |

The equal-armies test holds at 200 and at 2,000 a side, also with the sides
swapped. The flank wins 46 to 48 of 50, 19 to 25 more than frontally, over
four sets of 50 seeds. The first bar, a margin of 25, could not be met: with the
frontal mirror near 25 of 50 it asked the flank to win every battle, so the
test now asks for 40 of 50 and a margin of 15.

**Cost.** Measured on a shared 4-core VM, timings vary about 15% from run to
run. The yardstick is 50 squads of 40 a side, a 6.4-minute battle:

| Measure | Budget | Measured |
| --- | --- | --- |
| ms per battle second, one core | ≤ 1 | 0.85 (best of 6) |
| ns per soldier per substep | ≤ 60 | 53 |
| one battle | ≤ 2 s | 0.33 s |

The soldier controllers and blows take 55% of the time, apply and the grid
15% each, the squad controllers and the recount 7% each. What brought it
within budget, from 1.24 ms and 77 ns on the same machine:

- A soldier standing in their slot with nobody to fight, or out of contact
  and within half a pace of it, does not look for friends to push off. The
  formation keeps them apart. This is the one change that moves soldiers:
  `--compare` over 200 seeds stays within chance (`swordsmen*200` a side 93
  then 101 of 200, `swordsmen*400` 97 then 102, `cavalry*20 vs archers*30`
  186 then 182).
- A soldier's blow is rolled in the same pass as their plan, and only blows
  that reach somebody are kept, in soldier order.
- The grid keeps a copy of each soldier's position in cell order, so a search
  reads positions in a row, and builds in one list with no clearing.
- Neighbour searches skip candidates beyond their radius before any other
  work, and use a closure rather than an iterator chain.
- A squad that is not in contact does not tally whom it fights.

## Step 3: live battles on soldiers

- `battle.live_model` in `models.ron` picks the model for battles the player
  watches and commands, apart from `battle.model`, which still fights
  auto-resolved raids, the planner's forecasts and `--battle`. It defaults to
  `Soldiers`, so watched battles fight soldier by soldier while auto-resolve
  stays on squads (`prepare_live_battle`).
- `LiveView` carries every soldier of each squad on the soldier model
  (`soldiers`, empty on the squad model): where they stand, the way they face,
  and whether they stand, run, lie fallen or have left the field.
- The squad's place in the frame is its soldiers' centre, so the panels and
  everything that reads the frame keep working.
- **3D:** each figure of a live battle stands where its soldier stands, facing
  their way. The fallen lie where they fell, runners grey, and those who left
  the field are gone. Replays keep the block formation until step 4.
- **2D:** zoomed in (below a zoom of 0.3, where a metre of field is over a
  pixel), every soldier is a dot where they stand. The fallen are a small
  cross, runners grey and blinking. Zoomed out, a block per squad at its
  soldiers' centre, as before. "Look at the field" zooms in that far, and the
  zoom now goes down to 0.05.
- Both clients take `--shot FILE --shot-battle S`: the screenshot waits for a
  live battle fought S battle seconds and looks at its field. The 3D client
  also takes `--order TEXT`, as the 2D one does, so a raid can be scripted.

## Steps 4 and 5: soldier replays, and orders for RTS control

**Replays.** A frame of a battle on the soldier model holds every soldier
(`ReplayFrame.soldiers`, empty on the squad model), so the live view and
replays share one form:

- each soldier is a `FrameSoldier` of six bytes: position to a tenth of a
  metre (two `i16`, so ±3.2 km), facing to 1/256 of a turn, and state;
- frames are still recorded every 5 battle seconds (`run_recorded(5.0)`);
- the clients play a soldier between two frames along the line, turning the
  short way (`Replay::soldiers_at`). The 3D client walks a figure per
  soldier (battles.rs `play_battles`, not `crowd.rs`: the battle figures are
  their own entities, and a replay holds a few hundred soldiers at once). The
  2D client draws dots when zoomed in, as for a live battle.

**Size.** Replays are not in saves: the battle log lives in memory and goes
to the view, and `save.rs` writes the simulation's entities only. So
`SAVE_VERSION` stays at 16. In memory, a soldier replay costs
6 bytes × soldiers × frames, plus 24 bytes per squad per frame. Measured
with `cargo test -p sim_core replay_size -- --ignored --nocapture` (seed 1):

| Battle | Soldiers | Battle time | Frames | Soldiers' bytes |
| --- | --- | --- | --- | --- |
| a raid, `swordsmen*30 vs spearmen*20` | 50 | 4.9 min | 60 | 21 KB |
| `swordsmen*200 vs swordsmen*200` | 400 | 6.9 min | 85 | 208 KB |
| the yardstick, 50 squads of 40 a side | 4,000 | 6.4 min | 78 | 2.1 MB |

The battle log keeps the newest 16 (`BattleLog::KEEP`), so even 16 yardstick
battles stay near 33 MB. Auto-resolved raids are on the squad model by
default (`battle.model`), so a default game records no soldiers at all.

**Orders.** Step 5 first added `Flank` and `Form` as orders of their own.
They were replaced before it was merged by an order API that the mouse
controls of an RTS (Total War) map onto directly (module `battle::control`,
"Battle controls" below). A squad takes a **list** of orders and carries
them out in turn (`Battle::set_orders`):

- `MoveTo { anchor, facing, width, run }`: go to `anchor` (the centre of
  the front rank), and stand there facing `facing`, `width` metres wide,
  from now on. `None` keeps the way it came and the width it has. A move it
  has reached (within 1 m) gives way to the next order.
- `Attack(j)`, `Shoot(j)`, `Hold`, `Withdraw` and `Auto`, as before.

Going round an enemy and taking a formation are no longer orders. They are
ways of making moves:

- **Flank** is `LiveView::flank(q, j)`: a move out to a lane 20 m clear of
  `j`'s end on the side nearer `q` (at a slant, halfway along, if `q` is still
  in front of it), a move to that lane a little behind `j`'s front, facing in,
  then `Attack(j)`. With the mouse, the same is shift-queued right-clicks
  and a right-click on the enemy.
- **Form** is `LiveView::reform(q, files)`: a move to where the front stands
  now, `files` wide, then the orders it had. With the mouse, it is a
  right-drag in place. The panels offer four fronts (`LiveView::FRONTS`):
  column (4 files), deep (10), line (20) and wide (40).
- The squad controller still picks the shape. Riders charge in a wedge,
  spears brace, a long march goes in column unless the player set a width,
  and a squad stands loose if it is in loose order. The formation is no
  longer part of the API.
- On the squad model a move goes to `anchor` and ignores the facing and the
  width, since a disc has neither. The panels do not offer the flank or the
  fronts there (`LiveView::on_soldiers`).

Both clients also take `--battle LINEUP`, fought on `battle.model`, and
`--set PATH=VALUE`, as the CLI does: `--set battle.model=Soldiers --battle
"swordsmen*60 vs spearmen*40"` plays a soldier replay at start.

## Battle controls

Battles are to be fought with standard RTS controls, as in Total War. This PR
adds the sim's side of all of it (`battle::control`), and the clients' panel
buttons sit on it. The clients' mouse controls come next, in two PRs, each in
both clients.

| Feature | Control | What the sim gives |
| --- | --- | --- |
| Select squads | click, shift-click, box-drag, control groups | (client only) |
| Move, attack | right-click ground, right-click an enemy | `Order::MoveTo`, `Attack`, `Shoot` |
| Set the line | right-drag: position, facing and width | `MoveTo { anchor, facing, width }` |
| Run | double-click | `MoveTo { run: true }` |
| Group move keeping formation | right-click with several selected | `LiveView::group_shift`: each squad keeps its place, facing and width |
| Group line | right-drag with several selected | `LiveView::group_line`: one line along the drag, each squad a share by its front |
| Queued waypoints | shift-right-click | the order list: moves give way in turn |
| Pause to give orders | pause | the battle steps only when told (`Battle::step`) |
| Stances: fire | archers skirmish or hold fire | `Stance.fire`: `AtWill`, `Hold`, `Skirmish` (back away from enemies within `battle.skirmish_distance`, 40 m) |
| Stances: order | loose or tight | `Stance.spacing`: `Own`, `Tight` (shoulder to shoulder, stands up to a charge), `Loose` (`soldiers.loose_spacing`: fewer are hit where an arrow lands) |
| Pursuit of routers | kill, take prisoners, let go | `Stance.pursuit`: `Kill`, `Capture`, `LetGo`. A squad that lets go strikes nobody who runs and chases nobody; one that captures chases routers and takes them instead of striking them (`Member.captured`), and they leave the fight |
| Deployment | before the battle, in your own half | `Battle::deploy` (or `Command::Deploy`): only before the first step, only in `deploy_zone` (your half; defenders behind a wall stay inside it) |
| Ghost line | the dragged line, with arrival time | `LiveView::preview(q, orders)`: every soldier's slot where the moves leave them, and the seconds to get there at the squad's pace now (`LiveView.paces`) |
| Matchup hints | hover an enemy | `LiveView::matchup(a, b)`, `Kit::against`: health per blow and per arrow, per damage type, after armour and shield, and per second of one fighter |
| Morale and fatigue overlay | a toggle | `LiveView.condition`: each soldier's morale and fatigue (soldier model only; the squad model has `LiveView.morale` per squad) |
| Town doctrine | governance: cautious, balanced or aggressive, and a prisoner policy | `Doctrine { posture, prisoners }`, `Battle::set_doctrine(side, ..)`: on their own judgment a cautious side holds until an enemy is within `battle.cautious_wait` (120 m), an aggressive one runs in from twice `charge_distance`; every squad of the side takes `prisoners` as its pursuit |

Messenger delay for orders was considered and is not planned.

The clients tell a squad what to do with one call, `command_live_battle(world,
squad, Command)`, where a `Command` is `Orders(list)`, `Stance(stance)` or
`Deploy { anchor, facing, width }`. It returns an error when the command is not
taken: the squad is not the player's, the battle has started, or the point is
outside the deployment zone.

On the squad model, every control does the nearest thing a disc can:

- a move runs if told to, and its facing and width change nothing;
- the spacing stance changes nothing;
- holding fire, skirmishing, pursuit and doctrine work as on the soldier
  model;
- a preview lays out a ghost from the squad's centre.

**Build order.**

- **A.** In both clients:
  - mouse control: selection, control groups, right-click and right-drag,
    double-click to run, group moves, and pause to give orders;
  - stances in the panel;
  - the ghost line with its arrival time;
  - pursuit, including prisoners who become captives in the town economy
    (the sim's `Capture`);
  - matchup hints;
  - the morale and fatigue overlay.
- **B.** In both clients:
  - queued waypoints (shift-right-click);
  - the deployment phase before a live battle starts;
  - town doctrine in governance, saved with the town (a `SAVE_VERSION`
    bump), and used by auto-resolved battles.

## Where build A stands

Build A is in both clients, on shared logic in `battle::rts` (tested there
headlessly: selection, groups, the click and drag to command mapping, the
ghost, stances, matchup words and the overlay's colour). Each client only
turns the mouse into field metres and draws.

**Controls.** While a live battle waits for the player:

| Input | Does |
| --- | --- |
| left-click, shift-click, drag a box | select your squads (empty ground: none); Esc selects none |
| right-click | with squads selected: move there (several keep their formation, `group_shift`), or attack the enemy squad under it (archers with arrows shoot) |
| right-drag | with squads selected: lay out the line along the drag (one squad: `MoveTo` with its width; several: `group_line`); the ghost and its arrival time follow the pointer |
| double right-click | the same, running |
| Ctrl+1–9, 1–9 | make the selection a group; select it, and a second tap within 0.4 s looks at it |
| Z, X, C / V, B, N | with squads selected: fire at will, hold fire, skirmish / tight, loose, own order |
| O | the morale and fatigue overlay |
| hover an enemy squad | the matchup hint, e.g. "weak: their mail turns your blades" |
| Space | pauses the battle; orders still go in |

Pursuit (kill, capture, let go) is in the battle panel only, beside the
stance buttons, which show the selection's stance.

**Conflicts with the camera and other keys.** The battle reads input
before anything else (`PreUpdate`) and clears the presses it takes. Without
a selection, or off the field, right-drag still turns the 3D camera, and
the stance letters do what they always do (2D: C culture; 3D: X tilt-shift,
C the clock, V overlays, B bloom, N glow). The number keys are the battle's
while it waits (otherwise 2D game speed, 3D shader views), and O too (3D
outlines). WASD, the wheel and middle-drag keep moving the camera.

**Prisoners.** A capturing squad marks routers it reaches as captured
(squad model: a share of its front per second; soldier model: a blow that
takes instead of striking). When a live battle is settled
(`take_prisoners`), each captured soldier of the losing side changes sides:
their kit goes to the winner's armory, they leave their squad and their
raid. Taken by a winning raid, they march home with it as `Captive`, as
raid captives do, and become the town's people there; taken by winning
defenders, they join the town at once (there is no town-held captive state
to keep them in). Auto-resolved battles take no prisoners until doctrine
(B) gives them a prisoner policy.

## Open questions

- Should auto-resolved raids ever use the soldier model, or only battles the
  player watches? It depends on the step 6 measurement.
- Should friendly fire from arrows be on by default?
- Should a soldier who is down but alive be carried off (captives, or the
  wounded saved by their side), or does that wait for the aftermath?
