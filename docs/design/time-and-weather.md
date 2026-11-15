# Time, seasons and weather

Step 1 of the economy redesign (items, time, needs). The full design note is a
Claude Doc: "Econoplex economy redesign: items, time, needs".

## Clock

- A year is `time.months_per_year` simulated days (12). Each simulated day is
  the representative day of a month and stands for `time.days_per_month` days
  (10, so a year is still 120 days of economy and yearly totals stay put). The
  economy runs in steps through it (below); there is no loop over the days
  of a month.
- `SimClock.tick` counts simulated days (months): `year()` is
  `tick / months_per_year`, `day_of_year()` is the month. Agents' ages are in
  months (`AgentStore::age_months`).
- `time.steps_per_day` (300 by default, at least 1; `--set
  time.steps_per_day=N`) cuts the simulated day into economy steps, and the
  whole economy runs every step (`EconomyStep`): work, production, market,
  prices, eating, choices, migration, the military and governors.
  `SimClock.step` is the step within the day, `SimClock::steps()` the step
  count since the start (every system's RNG stream is seeded with it). A step
  stands for `days_per_month / steps_per_day` days of time (1/30 day, 48
  minutes, at the defaults); `Calendar::days()` is that, `Calendar::steps(d)`
  turns a span in days into steps.
- Season and weather are those of the middle of the month: the continuous day
  is `(tick - 1) * days_per_month + (days_per_month - 1) / 2 + hour / 24`. The
  ground (snow, wet soil) steps `Calendar::days()` days a step, so it melts and
  dries over the month.
- Work follows the hour of the representative day: indoor work runs 6:00 to
  18:00 (the equinox hours), outdoor work (nature, fields, building sites,
  new land, marching) from sunrise to sunset, 8 hours in midwinter and 16 in
  midsummer (`env::work_hours`). A step does the share of a working day that
  falls in its hours times the days it stands for
  (`env::work_days_this_step`), so a day of steps does a month of work, and
  night steps do none.
- `Simulation::step()` runs a whole simulated day; `Simulation::step_tick()`
  runs one step.

### What runs when

| Cadence | What |
| --- | --- |
| every step | needs and eating, choices of the idle, team assignment, work and batches, pay, market, prices, caravans, loans, the military's march and fights, deaths of the sick, the residents' list |
| once per day of time (`SimClock::day_of_time()`, every `steps(1)` steps) | body (capacity, training, disuse), the household ladder's wear, fuel, cold and happiness, social ties fading, culture following neighbours, random contacts |
| first step of the simulated day (`env::day_start`) | ageing, births, partnering, hazards, starving counts, governors, metrics, history samples, goals, the ledger's year end |
| every N months, on the first step (`SimClock::every_months(n)`) | trade rounds, organize, muster, raid plans, tribute, migration |

The slow processes moved to once per day of time are those whose state
changes only by a few hundredths a day: running them every step cost about a
third of a step's time and changed nothing measurable.

### Batches and the progress bar

Every workplace runs discrete batches (`Task`, saved and hashed). A batch
takes its inputs in whole units when it starts, gains progress every step
from the work done (`team_size^0.85 * work days`), and delivers its whole
output in whole units when progress reaches `required`; then the next batch
starts. `required` is the recipe's duration times the runs in the batch, and
the runs are as many as the team does in one working day of the step
(`floor(n^0.85 * work_step_days / duration)`, at least one), so yearly output
is that of the content's recipes. Work past the end of a batch carries over
into the next one (`AgentStore::carry`), so no work is lost at any step size.
People who take up a job join a batch under way with room in its team before a
new one starts. The view snapshot gives each building's progress
(`BuildingView::progress`, 0..1) and both clients draw it as a bar over the
building.

### Real time in the clients

Both clients run steps from real time: the speeds are 5 minutes, 1 minute and
12 seconds of real time per simulated day, then 1 and 4 days a second (the 3D
client also 12, 30, 60 and 120). At the default 5 minutes a day, 300 steps a
day come about once a real second. The labels come from
`calendar::speed_label`.

- The clients show dates as year and month (`calendar::date`, "y3 Mar").
- Outdoor work (nature, buildings, new land; then `work_hours: Daylight`, now
  always) is scaled by today's daylight against the equinox: 8/12 in midwinter, 16/12 in midsummer.
  Against the full working day all year (`AllDay`, then the default, removed
  2026-11-15), over 6 seeds and 20 years: population 1918 to 1876, food per
  head 8.9 to 6.1 days, swords 1373 to 1046, no starvation either way.
- Hosts that run the schedules themselves call `sim_core::add_schedules` and
  run `SimTick` once per step (the 2D game does).
- The calendar year starts at the spring equinox (`year_starts_at: 0.25`).
- Daylight is 12 hours at the equinoxes, 8 in midwinter and 16 in midsummer,
  with 2 hours each of dawn and dusk (`day_length`).

## Converting daily rates to the step

All conversions go through `sim_core::calendar::Calendar` (built from `time`,
`ModelParams::calendar()`, `SimClock::calendar()`):

| Kind | Helper | Rule |
| --- | --- | --- |
| a daily flow | `flow(x)` | `x * days` |
| a daily share (0..1) | `share(r)` | `1 - (1 - r)^days` |
| what a daily keep leaves | `keep(k)` | `k^days` |
| a moving average with a memory in days | `smoothing(m)` | `share(1 / m)` |
| a kick against relaxation | `kick(x, r)` | `x * share(r) / r` (same balance) |
| a time constant in days | `steps(d)` | `d / days` steps |
| a yearly rate | `per_year()` | divide by the steps in a year |

`days` is the days of time a step stands for. Work done in a step is the
working days that fall in its hours (`Calendar::work_share`,
`work_step_days`).

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
| culture memory | decision/culture.rs, social.rs | `steps(memory_days)` steps |
| price classes: speeds, max daily move, memory | pricing.rs | times days; `months` |
| legacy price adjust | market.rs | `flow` |
| scarcity cap | market.rs | `(1 + r)^days` up, `keep` down |
| input and goods use | market.rs | per day (divided by days), `smoothing(30)` |
| firm input stocks, short days | market.rs | per day; `days()` a step |
| loans: interest, instalments | firms.rs | yearly rate per step; spread over the `steps(days_left)` steps left |
| firm payout, import repayment | market.rs | `share` |
| bills, ledger figures | firms.rs, ledger.rs | kept as daily averages of the month |
| food reserve | decide.rs, models.ron | `food_reserve_months` (was `food_reserve_days`) times a month of hunger |
| job places | decide.rs | stock-limited places divided by days (a worker uses a month of inputs) |
| apprentice share reconsidered | decide.rs | every `steps(30)` steps |
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

`sim_cli --years 1 --weather weather.csv` writes one row per step and town.

![One year of sim weather in Ironhold](../images/weather-year.svg)
