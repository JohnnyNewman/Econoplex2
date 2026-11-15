# Time, seasons and weather

Step 1 of the economy redesign (items, time, needs). The full design note is a
Claude Doc: "Econoplex economy redesign: items, time, needs".

## Clock

- A year is `time.months_per_year` simulated days (12). Each simulated day is
  the representative day of a month and stands for `time.days_per_month` days
  (10, so a year is still 120 days of economy and yearly totals stay put). The
  economy runs once on it; there is no inner loop over the days of a month.
- `SimClock.tick` counts simulated days (months): `year()` is
  `tick / months_per_year`, `day_of_year()` is the month. Agents' ages are in
  months (`AgentStore::age_months`).
- `time.ticks_per_day` (12 by default) still cuts the simulated day into ticks,
  and `SimClock.sub` is the tick within it. They are kept because the clients
  show hours, daylight, dawn and dusk on the representative day; outdoor work
  is scaled by its daylight. The economy runs on the day's first tick. The
  other ticks only move the hours and the weather on, so the state hash is the
  same at any number of ticks per day.
- Season and weather are those of the middle of the month: the continuous day
  is `(tick - 1) * days_per_month + (days_per_month - 1) / 2 + hour / 24`. The
  ground (snow, wet soil) steps `days_per_month / ticks_per_day` days a tick,
  so it melts and dries over the month.
- `Simulation::step()` runs a whole simulated day; `Simulation::step_tick()`
  runs one tick.
- The clients show dates as year and month (`calendar::date`, "y3 Mar").
- Outdoor work (nature, buildings, new land; then `work_hours: Daylight`, now
  always) is scaled by today's daylight against the equinox: 8/12 in midwinter, 16/12 in midsummer.
  Against the full working day all year (`AllDay`, then the default, removed
  2026-11-15), over 6 seeds and 20 years: population 1918 to 1876, food per
  head 8.9 to 6.1 days, swords 1373 to 1046, no starvation either way.
- Hosts that run the schedules themselves call `sim_core::add_schedules` and
  `sim_core::run_day` (the 2D game does).
- The calendar year starts at the spring equinox (`year_starts_at: 0.25`).
- Daylight is 12 hours at the equinoxes, 8 in midwinter and 16 in midsummer,
  with 2 hours each of dawn and dusk (`day_length`).

## Converting daily rates to the month

All conversions go through `sim_core::calendar::Calendar` (built from `time`,
`ModelParams::calendar()`, `SimClock::calendar()`):

| Kind | Helper | Rule |
| --- | --- | --- |
| a daily flow | `flow(x)` | `x * days` |
| a daily share (0..1) | `share(r)` | `1 - (1 - r)^days` |
| what a daily keep leaves | `keep(k)` | `k^days` |
| a moving average with a memory in days | `smoothing(m)` | `share(1 / m)` |
| a kick against relaxation | `kick(x, r)` | `x * share(r) / r` (same balance) |
| a time constant in days | `months(d)` | `d / days` steps |
| a yearly rate | `per_year()` | divide by 12 simulated days |

What converts how (config units stay as written unless named):

| Quantity | Where | Conversion |
| --- | --- | --- |
| hunger per day, eating | needs.rs | `flow`; then meals until back at `eat_threshold` (at most 64) |
| fatigue | needs.rs | unchanged: the representative day's |
| starvation damage, health regrowth | needs.rs | `flow` |
| rent; rent stickiness | needs.rs | `flow`; `smoothing` |
| happiness relaxation | needs.rs, ladder.rs | `share` |
| ladder wear, fuel, cold damage, cold days | ladder.rs | `keep`, `flow`, `flow`, `whole_days` |
| ladder `interval`, shopping size | ladder.rs, models.ron | months (7 days became 1); units bought scale with the days |
| expected temperatures over the year | ladder.rs | `days_per_year()` |
| adaptive expectations | decision/expectations.rs | rate `share` |
| excess-demand wage step | decision/wages.rs | step times days |
| culture memory | decision/culture.rs, social.rs | `months(memory_days)` steps |
| price classes: speeds, max daily move, memory | pricing.rs | times days; `months` |
| legacy price adjust | market.rs | `flow` |
| scarcity cap | market.rs | `(1 + r)^days` up, `keep` down |
| input and goods use | market.rs | per day (divided by days), `smoothing(30)` |
| firm input stocks, short days | market.rs | per day; `whole_days` |
| loans: interest, instalments | firms.rs | yearly rate / 12; left over `ceil(months(days_left))` |
| firm payout, import repayment | market.rs | `share` |
| bills, ledger figures | firms.rs, ledger.rs | kept as daily averages of the month |
| food reserve | decide.rs, models.ron | `food_reserve_months` (was `food_reserve_days`) times a month of hunger |
| job places | decide.rs | stock-limited places divided by days (a worker uses a month of inputs) |
| apprentice share reconsidered | decide.rs | every `months(30)` steps |
| labour max daily, building load | decide.rs | `flow`; `smoothing(30)` |
| team output rate | work/teams.rs | times days; harvests can draw on several sites |
| work progress, tool wear | work/mod.rs | times days; `uses` = days on the task |
| work mood | work/mod.rs | `kick` |
| practice, diffusion, forgetting | models.rs, learning.rs | compounded over the days of practice |
| posted pay, pay rate smoothing | work/pay.rs | `smoothing`, `share` |
| school fees and practice | learning.rs | `flow`; over `days()` |
| body capacity, training, disuse | body.rs | over days; `share` |
| soldiers' pay, drill, march speed | military | `flow`; over `days()`; `flow` |
| war planner's year | military/planner.rs | `days_per_year()` |
| migration knowledge fade | migrate.rs | interval times days |
| food trend | life.rs | per day, `smoothing(60)` |
| governor flows and memories | governor.rs | per day; `smoothing` |
| granary profile | granary.rs | `share` |
| regrowth, frost, rot | farming.rs | times days; `keep` |
| history samples, quiet spell | history.rs | `SAMPLE_MONTHS = 1`, `QUIET_MONTHS = 3` |
| starving share | metrics.rs | per person-month |
| ages, birth and death rates per year | store.rs, life.rs | `months_per_year` |
| intervals and cooldowns (partner, trade, organize, muster, raid, tribute, migration, governor) | models.ron | counted in months: 1, 1, 2, 1, 12, 3, 3, 3 |

## Weather

- `climate: Seasonal(...)` sets the averages: mean temperature, seasonal and
  daily swing, how often it rains and in which season most.
- `weather: Fronts(...)` is seeded 3D value noise drifting east across the map,
  so fronts form, pass over towns and break up. Where the field passes the rain
  line it rains, or snows below about 1 °C. Warm and cold spells last a few days.
- Weather is a pure function of seed, time and place (`env::weather_at`), so
  it can be asked for anywhere, for example under the 3D camera. Only the
  ground (snow cover, wet soil) keeps state; it is saved.
- `Constant(temperature)` and `Clear` turn seasons and fronts off for tests.

## Interface for the 3D view

`sim_core::env::Environment` has the same fields as the stand-in in
`game3d/src/env.rs`: `hour`, `season` (0 = midwinter, 0.5 = midsummer),
continuous `day`, and per town `weather { temperature, wind, cloud, precip,
haze }` and `ground { snow, wet }`. Wind is `(f32, f32)` in world x, y.

## Looking at it

`sim_cli --years 1 --weather weather.csv` writes one row per tick and town.

![One year of sim weather in Ironhold](../images/weather-year.svg)
