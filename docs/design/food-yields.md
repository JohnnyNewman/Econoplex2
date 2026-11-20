# Food yields (plan Phase 2.2)

This note works out the food chain on paper (grain to flour to bread) and
compares it with what the sim does. It then sets the farm, mill and bakery
yields so that the two agree and the town has a surplus. The goals:

- the town's farmers feed it, with a surplus that fills the reserve in a
  normal year;
- 2-3 months of food per head in reserve at the end of winter;
- under 1 % of residents' months starving;
- `market.stop_at_floor` stays off (the owner's call), so the surplus has to
  go somewhere: into the granary, not into a glut.

## The calendar and the need

- **The year:** 12 months, each one simulated day standing for
  `days_per_month` (10) days: 120 days a year.
- **The need:** hunger grows by `hunger_per_day` (0.12), and a meal takes the
  portion that stills it. A person eats 0.12 food a day, 1.2 a month and
  14.4 a year.
- **Food values:** bread 1.0 a loaf, ale 0.7 a jug, grain 0.3 a unit eaten
  raw. People eat the best food on offer first (bread, then ale, then grain).
- **A month of food per head** is 1.2 food: 1.2 loaves, or 4 units of raw
  grain.

## The chain on paper

| step | recipe | per run | team (`max_team`) | per worker-day before | per worker-day now |
|---|---|---|---|---|---|
| farm | `farm_grain`: field -> 4 grain | 1 -> 1.5 person-days | 4 (4^0.85 / 4 = 0.81 runs per worker-day) | 3.25 grain | 2.17 grain |
| mill | `mill_flour`: 3 grain -> 3 flour | 1 -> 0.5 person-days | 3 (0.85) | 2.5 flour | 5.1 flour |
| bake | `bake_bread`: 2 flour + 1 wood -> 4 bread | 1 -> 0.5 person-days | 3 (0.85) | 3.4 bread | 6.8 bread |

- **What one grain is worth:** 1 grain -> 1 flour -> 2 loaves, so grain
  baked is worth 2 food against 0.3 eaten raw. One unit of wood bakes 4
  loaves.
- **A person's year of bread** (14.4 loaves) takes 7.2 grain, 7.2 flour and
  3.6 wood.
- **The labour for it:**

| step | before | now |
|---|---|---|
| farm | 2.2 worker-days | 3.3 |
| mill | 2.9 | 1.4 |
| bake | 4.2 | 2.1 |
| all | 9.3 | 6.8 |

  A resident does about 31 worker-days a year (children, rest, nights and
  soldiers included). So a person's bread is 30 % of the work before and
  22 % now.
- **Daylight:** outdoor work follows daylight (`time-and-weather.md`): 16/12
  of a working day in midsummer, 8/12 in midwinter. Farm work comes in the
  growing season, so a farmer does more than a working day's work on a
  summer day. The figures above are per worker-day of work done.
- **Losses:**
  - Standing crop rots on wet ground (2 % a day) and in the cold (1 %), and
    frost takes 5 % a day of a growing crop.
  - Food in the market spoils at `food_spoilage` (50 % a year).
  - Food in the granary spoils at a quarter of that (12.5 % a year), so a 3
    months' reserve loses about 3 % before it is eaten.
  - There is no storage limit.

## Places: where the old chain stopped

- **Bakeries:** a bakery has 16 places and every town has one.
  - Before: a town of 620 needs 620 x 4.2 / 120 = 22 bakers a day to bake
    everyone's bread, so 16 places bake 16/22 of it, about 10.5 loaves per
    head a year. The sim baked 10.7.
  - Now: 6.8 loaves per baker-day needs 11 bakers a day, so the places no
    longer limit bread. The slots stay at 16.
- **Mills:** milling needed 15 of a mill's 16 places before, and needs 7 now.
- **Grain:** the bread that could not be baked was eaten as raw grain (half
  of all grain) or rotted. Grain piled up at its floor price, and that was
  the glut: 9 of the 14 points of `glut%` were grain, 2-3 were flour.

## What the sim does, per seed

10 steps a day, 8 seeds x 30 years, the last year. All figures are per head
a year, except worker-days, which are a share of all work.

| | farm share | food chain share | grain per farmer-day | grain | bread | eaten | grain eaten raw | grain spoiled | bread per baker-day |
|---|---|---|---|---|---|---|---|---|---|
| before (mean) | 28 % | 49 % | 3.22 | 27.9 | 10.7 | 14.2 | 52 % | 18 % | 3.1 |
| now (mean) | 33 % | 49 % | 2.15 | 22.2 | 14.7 | 14.2 | 35 % | 14 % | 6.2 |

Per seed, now (seeds 42 to 49):

| seed | pop | farm share | grain | bread | grain eaten raw | granary grain at year end |
|---|---|---|---|---|---|---|
| 42 | 2814 | 33 % | 22.4 | 13.3 | 41 % | 11.4 |
| 43 | 2152 | 33 % | 24.7 | 14.8 | 28 % | 10.1 |
| 44 | 2731 | 38 % | 29.0 | 14.1 | 35 % | 11.7 |
| 45 | 2547 | 36 % | 19.3 | 12.2 | 42 % | 7.4 |
| 46 | 2899 | 32 % | 21.4 | 19.5 | 21 % | 9.9 |
| 47 | 2492 | 31 % | 15.5 | 11.6 | 39 % | 6.6 |
| 48 | 2738 | 33 % | 24.0 | 17.5 | 36 % | 11.1 |
| 49 | 1482 | 30 % | 20.9 | 14.4 | 36 % | 13.6 |

- **Paper and sim agree:**
  - Grain per farmer-day: 2.15 against 2.17 on paper.
  - Grain per head: the farm share (33 %) x 31 worker-days x 2.15 = 22 grain,
    against 22.2.
  - Bread before: the bakery limit gives 10.5 loaves, against 10.7.
- **The surplus:** the farmers grow three times the grain the town's bread
  needs (7.2 a head). Of the rest:
  - about a third is eaten raw where a town's bakers fall short;
  - 14 % spoils;
  - the granary keeps 9-10 grain a head through the winter.

## The share of farmers

- **It does not follow the yield.** Across a 2.5-fold range of farm yields
  (run duration 1 to 2.5 days) the farm share stayed at 27-35 % of
  worker-days, and the food chain at 46-49 %.
- **Why:** a trade is kept for life (Phase 3, step 1). The young take their
  parent's trade or the open trade that pays best when they enter, so
  scarcer grain draws few new farmers in a generation, and none of the
  adults.
- **Yields can't buy a medieval share of farmers.** Smaller yields only
  shrink the town:

| run duration | pop | starving | food per head |
|---|---|---|---|
| 1.5 days | 2482 | 0.18 % | 6.3 |
| 2 days | 1803 | 0.80 % | 4.0 |
| 2.5 days | 870 | 2.1 % | 7.7 |

  Medieval Europe had three in four people or more on the land. A share like
  that is for the careers work (Phase 3), not for the yields.
- **The yields chosen:** at 1.5 days a run, today's third of the work in
  farming feeds the town with a surplus, and the faster mill and bakery turn
  the surplus into bread.

## The reserve at the end of winter

The year closes at the spring equinox, so the year-end figures are the end
of winter. Food then sits in four places:

- **The granary:** the governor sets `granary_days` (40, now written in
  `models.ron`). It holds 2.5-2.7 months of food per head at the end of
  winter, the goal's 2-3 months.
  - It is mostly grain, counted at its raw value (0.3). Baked, the same
    grain is worth more than six times as much, so the reserve as bread is
    far larger.
  - A 90-day granary was tried. It held 5.7 months but left the glut where
    it was (14.2 %), because the granary only takes food the market holds
    over its own target. The glut came from the bakeries' limit, not from
    the granary's size.
- **The market:** 4.5-6 months, mostly bread. Its own target is
  `food_target_per_capita` (3 food, 2.5 months). Bakers bake on past it,
  since the stop is at `glut_factor` x the target, and a quarter of the
  bread made spoils.
- **The larders:** households keep `larder_days` (14).
- **The firms:** they hold none with `firms: Treasury`.

The health measure `reserve` (in `econoplex-sim`) is all of these together:
8.5-8.7 months now, against 5-6 before. Of the plan's 2-3 months, the
granary alone holds 2.5-2.7, so it meets the goal; the market's bread and the
larders come on top.

- **Per seed** (30 years): 3.0-13.1 months at 300 steps a day, 4.1-13.6 at
  10. Even the leanest seed keeps about 3 months.
- **Band:** the band checks the 8-seed mean, so it is set from the means:
  6.5-10.5 months, the measured 8.5-8.7 with 2 months either side.

## Trade between towns

- **How caravans work:** a caravan moves a good when the buyer's price is
  above the seller's by more than the transport (0.28 per 1000 units of road,
  about 25 % between neighbours). It carries the seller's surplus over its
  target, up to what the buyer lacks.
- **Before:** after the harvest every town's grain sat at its floor price, so
  there was no margin and grain did not move. A hungry town still gets food
  through emergency imports (`imports`).
- **Now:** bread takes up the grain, and grain sits at its floor price in 5 %
  of town-months, against 32 % before. That leaves room for a margin when
  one town's harvest is poor.
- **Not measured:** caravan flows are not counted per good, so how much grain
  now moves is not known. Counting them would show it.

## Grain in later chains (Phase 4)

Other uses would take up surplus grain. None of these is added here:

- livestock feed: pigs, poultry, and cattle over winter;
- fodder for horses: mounts are kit, and no town breeds them yet;
- ale (exists: 3 grain + a pot + wood -> 5 ale) and, later, malting and
  distilling;
- porridge and pottage: a better raw use than 0.3 food a unit;
- seed grain as a cost of the next harvest;
- straw for thatch, bedding and fodder.

## Results

8 seeds x 30 years. Food chain and farm shares are of the last year's
worker-days; `reserve` is in months at the end of winter.

| run | pop | starve % | food/cap | div | swords | glut % | reserve | food work | farm work |
|---|---|---|---|---|---|---|---|---|---|
| 10 steps a day, before | 2431 | 0.21 | 3.88 | 25.9 | 172 | 14.1 | 6.1 | 48.7 % | 27.1 % |
| 10 steps a day, now | 2482 | 0.18 | 6.34 | 25.5 | 116 | 3.4 | 8.5 | 48.1 % | 33.0 % |
| 300 steps a day, before | 2605 | 0.11 | 3.32 | 24.8 | 193 | 8.4 | 5.1 | 48.8 % | 26.2 % |
| 300 steps a day, now | 2654 | 0.06 | 7.00 | 26.4 | 186 | 3.3 | 8.7 | 47.9 % | 32.9 % |
| 10, `stop_at_floor`, before | 2409 | 0.26 | 3.96 | 24.7 | 153 | 0.7 | 6.2 | | |
| 10, `stop_at_floor`, now | 2592 | 0.08 | 5.70 | 24.2 | 130 | 0.4 | 7.5 | 48.7 % | 32.8 % |
| 300, `stop_at_floor`, before | 2612 | 0.15 | 3.56 | 24.3 | 196 | 1.0 | 5.4 | | |
| 300, `stop_at_floor`, now | 2771 | 0.05 | 5.33 | 26.5 | 185 | 0.7 | 7.5 | 47.4 % | 33.1 % |

- **Glut:** with `stop_at_floor` off, glut falls from 14.1 % and 8.4 % to
  3.4 % and 3.3 %. It is now the same at both step sizes.
- **Starving:** it falls at both step sizes.
- **Food per head:** it rises from 3.3-3.9 to 6.3-7.0.
