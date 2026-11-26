# Step 16: life on buildings

Towns stood still: no smoke, mill sails frozen, buildings appearing out of
nowhere. Now three things show what the sim is doing in them.

## Chimney smoke (`src/smoke.rs`, `assets/shaders/smoke.wesl`)

Smoke rises from:

- **Hearths:** the chimneys of the two-storey houses (the richest tier). It's
  a thin pale wisp in summer (cooking), full in autumn and winter (heating).
- **Workplaces with a chimney, while they work** (`BuildingView::progress`
  is set): bakery pale, kiln grey, smithy, machine shop and furnace dark and
  thick.

It works the way rain does (step 11). Each town has one mesh of small quads,
one per puff, six puffs a plume. A puff's four vertices all hold:

| attribute | holds |
| --- | --- |
| position | the chimney top (world) |
| UV | which corner, −1..1 |
| colour | r how dark, g a random seed, b its slot in the plume 0..1, a workplace 1 / hearth 0 |

The vertex shader gives each puff an **age** that runs from 0 to 1 over 5–7
seconds and starts again: `fract(time / life + slot + seed)`. Since the
slots are spread evenly, a plume always has puffs of every age. With age,
a puff:

- rises 2.4–3.4 units;
- drifts with the wind (`fog.wind`), more the higher it gets (`age^1.4`);
- grows from 0.1 to 0.6 units;
- fades in over the first 8 % and out as `(1 − age)^1.6`;
- turns paler as it thins.

The quad faces the camera (the view's right and up vectors). The fragment
shader draws a soft disc with a lumpy edge, fogged like everything else.
Daylight (the sun's height), season and wind come from the shared buffer,
and time from Bevy's globals. The material never changes. A town's mesh is
rebuilt only when its chimneys change, or when a workplace starts or stops.

## Mill sails (`buildings.rs`, `turn_sails`)

A mill's sails are their own entity, a child of the mill at the hub. While
the mill works they turn, slowly in a calm and about a turn in two seconds
in a gale (`sail_speed`). When it stops they run down slowly. There are only
a handful of mills, so the CPU turns them: one transform each per frame.

## Building sites (`buildings.rs`, `spawn_site`)

The sim now reports buildings being put up (`TownView::sites`): the kind,
where it will stand (its order's place, else the town's next building
place) and how far the work has got. The 3D view draws the building's
walls risen to that point inside a timber scaffold (four poles, boards
every half unit), with a pile of timber and stone beside it. The roof goes
on in the last eighth. A site is baked like a building and redrawn at each
eighth of progress. When the building stands, the site gives way to it.
Houses keep clear of sites as they do of buildings.

## Not yet: ruins

The sim never destroys buildings (raids loot, they don't burn). Ruins would
need that first.

## Cost

Smoke: a few thousand vertices per town, alpha-blended, no textures. Sails:
a transform per mill per frame. Sites: a baked mesh each, redrawn eight
times while it's built.

## Exercises

- Make hearth smoke thicker in the evening, when people are home.
- Let a strong wind flatten the plumes: less rise, more drift.
