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
- Warmth: each day a person needs `comfort_c - (day's mean + house_c + clothes)`
  degree-days of heat, shared by `people_per_hearth`. Fuel burns as the cold
  asks; less than half the heat needed counts as a cold day, and cold costs
  health (`cold_damage` per day at 10 °C short with no fire).
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

No starvation in any. Happiness is on the ladder's scale (gated rungs), not
comparable with the old formula.

`examples/needs_audit [years] [seed]` prints satisfaction per need by month.
At seed 42, winters leave 40 to 60 % of people cold even with firewood:
charcoal sits at its price ceiling and firewood takes wood from the bakers
(food satisfaction falls from 0.79 to 0.72). Fuel is the first resource
conflict the ladder brings out; growing it is for the rulers (steps 6 and 7).
