# Needs ladder

Step 4 of the economy redesign (items, time, needs). Code: `crates/sim_core/src/ladder.rs`.
On by default since 2026-11-13: `market.consumption: Needs(...)` in models.ron, and job choice by the ladder (then `decision.model: Ladder`, now the only rule).

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
| Basket (then the default, since removed) | 1918 | 8.9 | 1373 | 36.8 | 0.70 |
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

`farming` (`farming.rs`; then `Seasonal(...)` and off by default, now the
only farming model, see below): crops (fields, flax,
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

The ruler's food rules (`granary.rs`; then `market.food_rules: Ruler` and off
by default, now the only food rule, see below) drops the two
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

A governor now forecasts: food made and eaten per day,
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

The market's count of tools and weapons is what its armory holds (then
`market.item_stock: Held`, now always); the old count (`Counted`, removed 2026-11-15) kept weapons
carried by soldiers in stock.
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

## Stock targets (town lever)

Order `stockpile TOWN GOOD AMOUNT` (0 drops it), in `Town.policy.stockpile`:
any good the town keeps in its stores against a siege or bad times (Nils:
stock targets are one of the main levers of local government). The market's
target stock rises by what the store still lacks, so prices and pay draw
production toward it; the store (`Town.granary`, `Holder::Granary`) fills from
the market's surplus and gives back when the market falls below half its own
target. For tools and weapons the armory is the store: the target is the
number kept unissued. `granary TOWN DAYS` stays as the food shorthand. The war
redesign's kit rule writes per-good entries here.

## Households' and workshops' own stock targets

Nils: households and workshops need stock targets too, to keep running and to
survive. Both off by default.

- Households (`Needs(.., larder_days: D, ..)`): a household keeps D days of
  food in its larder against bad times. Half of the savings need is then food
  at home (safer than coin when prices spike), so food bought ahead competes
  with fuel and goods by the same need-per-coin rule. 6 seeds x 20 years with
  the ladder: D = 14 gives pop 1821 (1853 without), happiness 0.46 (0.44),
  no starvation.
- Workshops (`Workshops(.., input_days: D)`): each firm keeps D days of the
  inputs it has been using in its own store (`Firm::inputs`, not for sale),
  bought ahead with the cash above its wage reserve, and works from it first.
  D = 10: pop 1862 (1869), capability 13.2 (12.6).
- Fuel and clothes were already planned `plan_days` ahead by households.

## The whole new economy at once

`assets/config/presets/new_economy.ron` switches everything on (needs ladder
with households and a 14-day larder, seasonal farming, the ruler's food rules,
daylight work hours, armory-held gear). Use `--preset FILE` with `sim_cli` or
the 3D view (`econoplex3d --preset assets/config/presets/new_economy.ron`).

6 seeds x 20 years against today's defaults: population 1914 -> 1446,
starving 0 -> 0.5 % of person-days, food per head 8.2 -> 3.2, glut 0.28 ->
0.03, happiness 0.69 -> 0.41 (ladder scale), capability 41.3 -> 40.0.
Leaving one option out (population): ruler's food rules 1803, needs ladder
1792, daylight work 1699, seasonal farming 1622, armory-held gear 1490. Each
makes the world harder, and together they compound.
## Why stocking less gear shrinks the economy (2026-11-13)

War thread finding: `military.kit_demand: Soldiers` (weapons stocked per
soldier, not per head) cuts population 1914 -> 1834 and food per head 8.2 -> 4.3
over 6 seeds x 20 years. The per-head rule (`PerCapita`) was removed on
2026-11-15, with the one-roll battles (`battles: Roll`), the fixed-length march
(`march: Fixed`) and the random raid draw (`decision: Appetite`); the numbers
below are history. Checked:

| 6 seeds x 20 years | PerCapita (since removed) | Soldiers |
| --- | --- | --- |
| swords | 984 | 200 |
| input use | 136 | 96 |
| household use | 87 | 76 |
| wealth Gini | 0.63 | 0.32 |
| raids / revolts | 29 / 4.3 | 34.5 / 7.2 |
| same, raids off: pop | 1966 | 1930 |
| same, raids off: food per head | 6.9 | 3.7 |

- Under `firms: Treasury` the town buys every output, so a high weapon target
  is a treasury-funded work programme: iron, coal, charcoal and smithing wages
  (input use -30 to -40 % without it). Less of it means less income, less
  household buying and much less inequality (smiths were the rich).
- Food per head falls from 2.7 to 1.4 times the market's food target (3.0): the
  lost part was surplus (glut 0.28 -> 0.09), not a shortage; nobody starves.
- Most of the population loss is war: with fewer swords in store towns lose
  more raids and face more revolts. With raids off the gap is 1.8 %.
- No bug in the stock logic. Under the ruler's rules the gap is wider (1848 ->
  1605): the treasury's sword spending also paid for the food levers.

## Stockpile targets follow the seasons

Nils: grain is harvested once a year, firewood is burned mostly in winter, so a
stockpile target should depend on the season, estimated from how stocks move.
Each town learns from its goods ledger how much of each good it makes minus
uses per day in each month (`Town::flow_profile`, updated daily, about two
years of memory). For a month, the store it needs is the deepest shortfall the
coming twelve months add up to; the target applies in full in the month that
needs most and down to a quarter (`SEASON_FLOOR`) in the month that needs
least. Goods made and used evenly keep their whole target.

Learned shares by month at seed 17 after 3 years (month 1 = spring equinox):

| | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| grain | .50 | .32 | .39 | 1.0 | .86 | .71 | .48 | .85 | .79 | .47 | .25 | .25 |
| wood | .70 | 1.0 | .67 | .72 | .87 | .50 | .41 | .29 | .25 | .36 | .36 | .25 |

The curves are noisy (raids, migration, price swings) and learned, not
designed; firewood is counted as used when households buy it (planned a month
ahead), not when it burns.

## The new economy is the default (2026-11-13)

Nils: always go with the more principled mechanism until it is time for
fine-tuning. `models.ron` now defaults to the needs ladder (with households and
a 14-day larder), seasonal farming, the ruler's food rules, daylight work and
armory-held gear (the `new_economy` preset is the same). State hash at seed 42
over 10 years: `7e49f13d8c7ee3bf` (was `fcf216872512dde5`). 6 seeds x 20 years:
population 1446, starving 0.46 % of person-days, food per head 3.2. Tests
calibrated to the old economy pin it with `legacy()` in `tests/simulation.rs`.

## Decisions by the ladder's derivative (`decision.model: Ladder`)

Nils: the derivative of a person's total satisfaction should give the decision
what to do next. `ladder::gradient(sat)` is the slope of the weighted
satisfaction per need: its own rung's gate, plus how raising it opens the rungs
above. Job choice under `decision.model: Ladder`: a day's work is worth its
expected pay (plus what the practice earns later) times the value of a coin
(the larger of a meal's worth of food for the hungry and savings for the fed),
minus the rest it costs times the rest slope; resting is worth the rest it
gives. `decision.ladder_scale` sets how sharply people pick the best option.
Purchases already worked this way.

First results, 6 seeds x 20 years on the new defaults (not the default yet):

| decision model | pop | starving |
| --- | --- | --- |
| Features (default) | 1446 | 0.46 % |
| Ladder, scale 200 | 947 | 2.95 % |
| Ladder, scale 1000 | 992 | 2.47 % |
| Utility | 407 | 3.92 % |

Better than the old utility model, worse than the feature weights. At seed 42
after 5 years, Ladder towns have fewer woodcutters (Greenvale none, against 17 %
of workers with Features), fewer fields and less farming in Ironhold (49 %
against 66 %). Next: find which term undervalues wood and fields (pay that the
price ceiling caps; no value yet for the town-wide need for firewood).

### Why ladder decisions starve more, and what helps (2026-11-14)

`examples/jobs_audit [years] [seed] [path=value ...]` shows what each job pays
and how many take it. At seed 42, year 3, Ironhold, with ladder decisions:
baking pays 17 a day but the two bakeries hold only 15 people (bread at its
price ceiling), flour and grain sit at their price floor (milling 0.20,
farming 0.88), and the rest crowd into woodcutting at 0.66. Ladder decisions
follow pay, so a price system that does not pass the bread shortage back to
flour, grain and new bakeries hurts them more than the feature weights.

Two principled fixes, both off by default for now:
- `market.pricing: Demand(..)` (already there): an input's target stock is
  what the trades that use it need, so a busy bakery raises flour's price.
- `market.building_value: Marginal(payback_days, max_factor)` (new): while a
  town's buildings of a type are full, another is worth its slots times how
  much more its best job pays than the town's typical job, over the payback
  days. Without the "full" condition towns overbuild (Ironhold: 101 buildings).

6 seeds x 20 years on main (kit for soldiers on):

| decisions | pricing | buildings | pop | starving |
| --- | --- | --- | --- | --- |
| Features | StockTarget | Load (then the default; both since removed) | 1140 | 0.88 % |
| Features | Demand | Load | 1143 | 0.82 % |
| Features | Demand | Marginal(10, 1) | 1121 | 0.90 % |
| Ladder | StockTarget | Load | 980 | 2.76 % |
| Ladder | Demand | Load | 1275 | 1.30 % |
| Ladder | Demand | Marginal(10, 1) | 1220 | 1.27 % |

With Ladder + Demand + Marginal as defaults, five tests fail: the first year
loses 7 % of its people (before the first harvest), government orders and
caravans move less, and workshops (as firms) collapse.

## Ladder decisions are the default (2026-11-14)

Nils: go with the new model. Defaults now: `decision.model: Ladder`,
`market.pricing: Demand(cover_days: 10, blocked_workers: 2)`,
`market.building_value: Marginal(payback_days: 10, max_factor: 1)`. The
building value now compares a new building's best pay with the lower quartile
of the town's pay (what the workers who would fill it earn now), not the
median. State hash at seed 42, 10 years: `b6667483965e194b`.

6 seeds x 20 years: population 1308 (1140 with the feature weights), starving
1.0 % of person-days (0.88 %), food per head 2.4, diversity 21.7 (29.6),
capability 27.7 (about 40): people follow pay, so towns make fewer kinds of
goods. The first winter is hard for the mining town (Ironhold loses about a
quarter of its people at seed 42); `first_year_has_no_famine` now asks that no
town lose a third and the world keep 90 %. Tests calibrated to the old models
pin them with `legacy()`, which now also sets Features, StockTarget pricing and
the Load building value.

## Steady-state start (plan Phase 2.1)

`start: Steady(food_days, input_days, raw_days)` (default; `start.rs`) starts
every town as if its economy had been running, with amounts worked out from the
recipes, needs and population (so they hold for an outpost of one household as
for a town of 300):

- Market food: `food_days` (25) of what the residents eat. A person eats a whole
  loaf when hunger passes the threshold, so about 0.2 loaves a day, not 0.12.
- Pipelines: walking each food recipe back, inputs for `input_days` (10) of the
  work that uses them (flour), goods harvested from nature for `raw_days` (90:
  grain to the next harvest, oven wood).
- Households: the larder target (`larder_days` of food), fuel for the planning
  days at the year's mean temperature for each home, and the clothes people
  can wear.

For 300 people: about 1,600 bread, 320 flour, 2,900 grain, 1,450 wood. Over 4
seeds and 15 years, the year-2 famine falls to about a third (starving samples
66 against 197 with the scenario's stock), but year 2 is still the worst year:
the labour shortage in milling and baking (food audit) is Phase 2.2.
The `Features` and `Utility` decision models have since been removed
(agent-decisions.md); `Ladder` is the only one. Under a consumption model
other than `Needs` it reads the ladder's default parameters.

## The old farming, food and work rules removed (2026-11-15)

Seasonal farming, the ruler's food rules and daylight work are the defaults
(since 2026-11-13), and the rules before them are now gone: `farming: AllYear`
(every nature site regrows the same amount every day), `market.food_rules:
Town` (seed grain always held back) and `time.work_hours: AllDay` (a full
outdoor working day all year). `Seasonal`, `Ruler` and `Daylight` are the only
variants left: the seed reserve is the ruler's order (`reserve TOWN on`), and
a governor always forecasts its food, takes stock on the first day, holds the
seed back and keeps a granary. The "Town rules" rows in the tables above are
the record of how the old food rules ran.

## Single-variant switches folded away (2026-11-15)

With the old variants gone, the switches that had one choice left are now
plain parameters or nothing at all, and the run is unchanged (same state
hash): `farming: (crops, ...)`, `market.pricing: (cover_days,
blocked_workers)` and `market.building_value: (payback_days, max_factor)`
keep their numbers; `decision.model`, `market.food_rules`, `market.item_stock`,
`time.work_hours`, `military.kit_demand`, `military.battles`,
`military.march` and `military.decision` are gone. The names in the sections
above are the record of the switches as they were.

## Meals are portions (plan Phase 2.2, first step)

Food is a stack, so a meal now takes the portion that stills the hunger
(`subsistence::portion`: hunger / food value, at most one unit) instead of a
whole loaf at the threshold. People eat what their hunger grows by (14.4 food a
year at 0.12 a day; the food audit measured 19 to 21 before). The steady-state
start sizes food by the same rule.

8 seeds x 20 years: population 1331 -> 1713, starving 0.92 % -> 0.43 %, food
per head 3.0 -> 4.7.

## Nobody sells below cost (plan Phase 2.2, second step)

The jobs audit showed flour at 0.66 times its base price while grain, in winter,
sat at five times: milling a sack lost money, so it paid nothing, and the
bakeries ran short of flour. Now a made good's price never falls below its cost
by its cheapest recipe: its inputs at today's prices plus a day's pay of the
town's poorer jobs (the lower quartile) for the work.

8 seeds x 20 years: population 1713 -> 1724, starving 0.43 % -> 0.32 %, food
per head 4.7 -> 5.3. Over 30 years food per head is 5.37 (health band raised).

## Savings as months of security (plan Phase 2.3)

The savings need is now: how many months of the lower needs (food, a share of
the fire, clothes wearing out, rent) the household could pay for at today's
prices from its coin and stores, scored ln(1 + months) / ln(13): 0.27 at one
month, 0.54 at three, 0.76 at six, 1 at twelve. With a larder target, half the
need is still food at home. Work is valued by the same slope (what one more
coin adds to the months). `savings_days` is gone.

8 seeds x 30 years: population 2376, starving 0.28 %, food per head 6.1.

## Trades (plan Phase 3, step 1)

A trade is now kept for life. The young enter one at `life.apprentice_age` (14)
as apprentices. They take the parent's trade if it has an open place, and
otherwise the open trade that pays best in town. Places are a hard limit: each
master takes at most `organize.apprentices_per_master` (3). Adulthood is at 16.
Rank follows years in the trade: an apprentice until adult, then a member, and
a master after `organize.master_years` (8). An adult who never apprenticed
joins the guild of their best skill once it reaches `guild_threshold`. A
teacher's lesson is shared: with n learners each gets
1 / (1 + `teaching_crowding` x (n - 2)) of it (crowding 0.3), so fewer
apprentices learn faster each.

8 seeds x 20 years: population 1583, starving 0.66 %. 8 seeds x 30 years:
population 2076, starving 0.47 %, food per head 5.6, diversity 24.9.

## Belonging and esteem pinned at 0.5 (2026-11-15)

Little in the content serves belonging or esteem yet, so households spent
savings chasing two rungs the economy could not supply. `fixed_needs` in
`LadderParams` now holds both at a neutral 0.5 (models.ron), as growth already
was. Goods earn nothing for serving a pinned need. Unpin a need once the
content serves it.

Default run, 8 seeds, 30 years (mean ± spread between seeds), measured on main
after meals as portions, the cost floor, savings as months and trades:

| | pop | starve% | food/cap | div |
| --- | --- | --- | --- | --- |
| before | 2208 ± 158 | 0.40 ± 0.21 | 5.31 ± 1.34 | 25.1 ± 1.51 |
| pinned | 2297 ± 126 | 0.25 ± 0.17 | 7.41 ± 1.88 | 26.0 ± 1.82 |

All results stay inside the health bands.
