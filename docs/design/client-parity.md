# 2D and 3D client parity

The 2D client (`crates/game`) is the test bench; the 3D client (`crates/game3d`)
is the showcase. Both draw the same simulation. This note lists what the 3D
client shows that the 2D client did not (as of 2026-11-15), ordered by how
much each helps testing, and how each gap was closed.

**Rule for later work:** a new feature counts as done only when it is visible
in both clients.

## Gaps, most useful for testing first

| # | What the 3D client shows | Why it matters for testing | 2D client |
| --- | --- | --- | --- |
| 1 | **Why? view** for a town (`sim_core::explain`): what is short, what stops it, where the money goes | Explains the AI governors' and the economy's choices without reading logs | `Y` report |
| 2 | **Person details** (`sim_core::view::person`): satisfaction per need, larder, household goods, all equipment slots, household | The needs ladder, households and gear are invisible otherwise | `I` report, follows the selection |
| 3 | **Food security and town needs**: food days, granary, seed reserve, starving and cold person-days, mean satisfaction per need, armory | Food is the ruler's challenge; famine and cold show up here first | `T` report |
| 4 | **Weather, season and day length** (`sim_core::env`) | Farming follows the seasons; cold drives fuel demand | `T` report, first lines |
| 5 | **History charts**: monthly series per town | Trends over years, not just today | `H` report, text sparklines |
| 6 | **Event log** longer than the five latest events | Following raids, famines and foundings over a run | `L` report |
| 7 | **Battle replays** beside the defending town (war thread) | Seeing how a battle went | Played back beside the defending town (battles.rs): squad blocks, red attackers, blue defenders, routs blink, outcome written; live battles with orders stay in the 3D client |
| 8 | **Culture overlay** (towns tinted by culture) | Migration and culture drift | `C` overlay: people in their own culture's colour, towns ringed in theirs |
| 9 | Day and night, people's daily routine, town lights, families sharing a house | Look and feel | Not planned: the 2D client steps whole days |

Already in both clients: town table, events (latest five), economy panel
(money flows, work, goods, shortages), policy levers and building placement,
agent selection and inspector (skills, mind, culture fit, physique), product
space overlay, trade flows, quicksave and quickload.

Money figures (treasury, prices, wealth) are left out of the new reports
until integer coin lands, so the two changes don't collide.

## The 2D reports panel

One panel in the middle of the screen shows one report at a time. Its key
opens it, and pressing the same key again closes it. Reports follow the
economy panel's town (`Tab`) and the selected person. Each report is computed
once per simulated day (or when its subject changes), not every frame.

| Key | Report |
| --- | --- |
| `Y` | Why? for the town |
| `T` | Town: weather and season, food security, needs, armory |
| `H` | History: population, food per resident, soldiers, buildings per town |
| `L` | Event log, newest first |
| `I` | Person: needs, larder, household goods, equipment, household (opens itself when you click someone) |
