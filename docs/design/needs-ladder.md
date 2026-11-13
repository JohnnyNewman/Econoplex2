# Needs ladder

Step 4 of the economy redesign (items, time, needs). Code: `crates/sim_core/src/ladder.rs`.
Off by default: `market.consumption: Needs(...)` (example in models.ron).

## Model

- Nine needs on five rungs: food, warmth, rest | shelter, health, savings |
  belonging | esteem | growth. Each person has a satisfaction 0..1 per need.
- A rung's weight is the product of smooth gates (0.55 to 0.85 by default)
  over the satisfaction of every rung below, so a cold or hungry person cares
  little for esteem.
- Goods declare what they serve in the content, `household.serves`:
  | Good | Serves |
  | --- | --- |
  | firewood | warmth, 15 degree-days per unit burned |
  | charcoal | warmth, 30 degree-days per unit burned |
  | clothes | warmth, 3 °C per piece worn (2 at once); esteem 0.15 |
  | ale | belonging 0.3, drunk over a week |
  | pot, cloth | shelter 0.1, 0.15 |
  | silver | esteem 0.5 |
- Households: a couple and their children share one home. Its keeper (the
  partner with the lower index) holds the fuel and home goods (pots, linen) and
  buys them for everyone at the hearth, valuing them for each person there;
  clothes, ale and silver stay personal.
- Warmth: each hearth burns `fire_per_c` degree-days per degree the room falls
  short of `comfort_c` after the house and the family's mean clothing, and
  `fire_per_extra_person` more per further person (a family of four burns 1.75
  times one person alone). Less than half the heat needed counts as a cold day,
  and cold costs health (`cold_damage` per day at 10 °C short with no fire).
- Shopping: every `interval` days a household buys, one unit at a time, the good
  that raises its weighted satisfaction most per coin. Money serves savings
  (days of food covered by money and larder against `savings_days`), so a
  purchase must beat keeping the money. No spending share, no wealth threshold.
- Fuel and clothes are planned `plan_days` ahead from the seasons everyone
  knows, with today's warm or cold spell fading over about five days.
- Food is still eaten by the hunger rule; its satisfaction is `1 - hunger`.
  Nothing serves growth yet (neutral 0.5).

## First results (6 seeds x 20 years)

| | pop | food/head | swords | capability | happiness |
| --- | --- | --- | --- | --- | --- |
| Basket (default) | 1918 | 8.9 | 1373 | 36.8 | 0.70 |
| Needs | 1906 | 8.0 | 1035 | 35.0 | 0.42 |
| Needs + daylight work | 1819 | 5.5 | 962 | 35.9 | 0.41 |
| Needs with households | 1884 | 8.9 | 911 | 35.5 | 0.43 |

No starvation in any. The first two Needs rows are from before households
(each person kept their own fire, a quarter of a hearth each). Happiness is on the ladder's scale (gated rungs), not
comparable with the old formula.

`examples/needs_audit [years] [seed]` prints satisfaction per need by month.
At seed 42, winters leave 40 to 60 % of people cold even with firewood:
charcoal sits at its price ceiling and firewood takes wood from the bakers
(food satisfaction falls from 0.79 to 0.72). Fuel is the first resource
conflict the ladder brings out; growing it is for the rulers (steps 6 and 7).

## Seasons in farming (step 6)

`farming: Seasonal(...)` (off by default, `farming.rs`): crops (fields, flax,
pasture) grow with the day's mean temperature (none below 4 °C, full from
14 °C), scaled so a year grows what the old daily rule did. A season's growth
stands in the field (`crop_capacity` × capacity) until reaped; frost damages
it, and it rots on wet ground and in the cold, so a crop not reaped by late
autumn is lost.

| 6 seeds x 20 years | pop | food/head | swords | capability |
| --- | --- | --- | --- | --- |
| default | 1918 | 8.9 | 1373 | 36.8 |
| seasonal farming | 1929 | 8.1 | 1415 | 35.9 |
| needs + households | 1884 | 8.9 | 911 | 35.5 |
| needs + households + seasonal farming | 1869 | 9.6 | 978 | 35.7 |

No starvation anywhere: the town-wide food rules (seed held back, farming
drawn by town food scarcity) still smooth the hungry gap. Turning those into
the ruler's levers is step 7.

## Food as the ruler's challenge (step 7, first part)

`market.food_rules: Ruler` (off by default, `granary.rs`) drops the two
town-wide food rules: seed grain is no longer held back unless the ruler orders
it (`reserve TOWN on`), and people are no longer drawn to farm by the town's
food scarcity, only by pay. New lever: `granary TOWN DAYS`, food the town keeps
out of the market (taken from a full market, released into a hungry one; it
spoils at a quarter of the market's rate). AI governors (first planner) turn the
seed reserve on, keep a 40-day granary, and keep their old food levers
(subsidies, imports, new fields).

With needs, households and seasonal farming on, 6 seeds x 20 years:

| | pop | starving (person-days) | food/head |
| --- | --- | --- | --- |
| Town rules | 1869 | 0 | 9.6 |
| Ruler, every town governed, granary only | 1499 | 0.6 % | 4.7 |
| Ruler, every town governed, granary + seed reserve | 1650 | 0.3 % | 6.4 |
| Ruler, nobody governs | 1259 | 1.2 % | 4.6 |

Where it starves (seeds 42-44, player town Greenvale ungoverned): mostly
Ironhold, the mining town, then Brassmoor; Greenvale, the farm town, barely
starves even unmanaged. The planner does not yet make food-poor AI towns safe,
and the player's farm town is too easy to make the challenge felt.

### Food planner (step 7b)

Under `Ruler` rules a governor now forecasts: food made and eaten per day,
averaged over a year (`governor.forecast_days`, from the goods ledger), and the
days its market and granary food would last at that rate. Below
`food_horizon_days` (90) it acts as if hungry (food subsidies, imports, a new
field) before the market runs dry, and it also takes stock on the first day.

| 6 seeds x 20 years, everything on, all towns governed | pop | starving |
| --- | --- | --- |
| Town rules | 1869 | 0 |
| Ruler, granary + seed reserve (7a) | 1650 | 0.3 % |
| + forecast | 1680 | 0.3 % |
| + first-day decision | 1740 | 0.2 % |

With the player town (Greenvale) ungoverned, it now starves in some seeds
(seed 44: 12,318 person-days) while the AI towns mostly get by; Ironhold, the
mining town, still has famines in its first months, before the first harvest.

## Items with owners (step 2b-1)

`goods.rs`: every item (tool, weapon; later armour, shield, mount) has an
`Owner(Holder)`. Holders: `Market(town)`, `Granary(town)`, `Workshop(town,
firm)`, `Home(keeper)`, `Person(i)`, `Armory(town)`, `Squad(SquadId)`. Where an
item is follows from its owner. Items move only through `to_armory` /
`from_armory`, `to_person` / `from_person` (by `Slot`: tool, weapon, armour,
shield, mount; content `equip.slot`), and `to_squad` / `from_squad`.
`goods::wear(item, hours)` wears an item by the hour (8 hours = one use of its
`durability`). `Squads` is a stub table for the war redesign (home town,
members, place, kit); `eats_from` says where a person eats (their squad while
marching, else their home); `carry_capacity` says what a holder can carry
(squads: 25 kg per member for now; goods have `kg` in the content).

`market.item_stock: Held` (off) makes the market's count of tools and weapons
what its armory holds; the old count kept weapons carried by soldiers in stock.
6 seeds x 20 years: pop 1918 -> 1913, swords 1373 -> 1332, raids 27.7 -> 22.5.

## Stacks (step 2b-2)

Simple goods are stacks held by a holder, each in the storage it already had:
market (`Town::stock`), granary, workshop (`Firm::stock`), a person or home
(larder for food, `owned` for the rest), and now a squad's packs
(`Squad::stacks`). `goods::move_stack(from, to, good, q, ..)` is the one way to
move a quantity between holders: no more than `from` holds and no more than `to`
can carry (`carry_capacity` against `load_kg`). Moves change no total, so the
goods ledger records nothing; the census now counts squad packs. People
marching with a squad eat from its packs first (`squad_meal`).
