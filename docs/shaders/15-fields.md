# Step 15: fields through the farming year

The fields biome used to be a fixed patchwork: every 6 × 6 plot was ripe
grain, young crop or stubble, all year. Now each plot goes through the
farming year, and the harvest follows the sim's.

## The year of a plot (`field_colour` in `terrain.wesl`)

Each plot hashes to a crop and a few traits of its own:

| hash | what |
| --- | --- |
| `kind` | 15 % fallow (rough pasture all year), 45 % grain, 40 % a green crop (part of it flax) |
| `late` | the direction of its rows (along x or z) and up to ±9 days of delay in the calendar |
| `turn` | its place in the harvest and in the autumn ploughing |

Through the year (`year` = 0 at midwinter, 0.5 at midsummer):

1. **Winter, early spring:** ploughed soil with furrows (snow from step 11
   on top).
2. **Spring (0.16 → 0.36):** green rows sprout and close up. Between young
   rows the soil shows: `gap` is 0 on a row and 1 between rows, and the crop
   covers `cover · (1 − gap)` early and everything once `cover` is near 1.
3. **Summer:** grain ripens from green to gold (0.4 → 0.56). Flax flowers pale
   blue for a few weeks in early summer. Other green crops stay green.
4. **Harvest:** a plot is reaped once its `turn` is below the share of the
   harvest its town has taken in (`reaped`): stubble, pale straw with faint
   rows.
5. **Late autumn (0.78 → 0.92):** the stubble is ploughed under, plot by
   plot.

Thin grass strips mark the edges of the plots. The fields no longer take the
meadow's seasonal colours (straw in autumn, brown in winter) from `seasonal`,
since they keep their own year. Wetness and snow still apply.

## Where the numbers come from

- **The calendar.** The year's fraction changes every frame, so it travels in
  the shared buffer (`fog.debug.y`) like the rest of the weather. No material
  changes.
- **The harvest** (`src/fields.rs`). The sim reports each town's standing
  crop as a share of a year's growth (`TownView::crops`, from its crop
  sites in `farming.rs`). The 3D view keeps each town's peak this year. After
  midsummer, `reaped = 1 − crops / peak`. That goes into the terrain uniform
  (`towns[k].w`, with `fields.x` towns), written only when a value moves by
  more than a 64th, a few times a season. A plot follows the town nearest
  to it.
- A town whose crop never stands (reaped as it grows), the view without a
  sim, and a season set by hand (`U`, `--season`: the sim's harvest belongs to
  its own date) fall back to the calendar's harvest: from late summer to
  mid-autumn.

So a town short of hands at harvest time keeps standing gold fields well into
autumn, which is exactly when the sim starts losing that crop.

## Cost

A loop over at most 16 towns and a few hashes per field fragment, no
textures. The CPU work is a few comparisons per frame.

## Exercises

- Make the rows follow the slope of the terrain instead of x or z.
- Give each town's fields their own crop mix from what its market sells.
