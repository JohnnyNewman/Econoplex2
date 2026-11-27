# 2D and 3D client parity

The 2D client (`crates/game`) is the test bench, and the 3D client
(`crates/game3d`) is the showcase. Both draw the same simulation.

**Rule for later work:** a new feature counts as done only when it is visible
in both clients.

This note compares the two clients on main at e3e0fa8 (2026-11-26). It is plan
Phase 8, step 1, and is read only: nothing here is built yet. It lists what
the 3D client shows or lets the player do that the 2D client does not.
Purely visual effects (shaders, outlines, bloom, fog, grass, figure
animation) are left out unless they carry information.

Groups 1 to 5 of the recommended order are done (2026-11-26, rows marked
*same (group 1)*, *same (groups 2–3)*, *same (group 4)* or *same (group 5)*),
so the 2D client now shows what the 3D one does, apart from the rows still
marked *partial* or *missing*. The other direction is a separate package.

The first round (2026-11-15) closed these gaps with the 2D reports panel:
the Why? view, the person details, food security and town needs, weather in
text, history, the event log, battle replays and the culture overlay (see
the end of this note).

## How to read the table

- **2D status:**
  - *same* means the 2D client shows the same information, perhaps drawn
    differently;
  - *partial* means it shows some of it;
  - *missing* means it shows none of it.
- **Effort:** *S* is under a day, *M* is a few days, and *L* needs a change to
  how the 2D client runs the simulation.

## Gaps by area

### Time and speed

| Feature | 3D file(s) | 2D status | Effort | How to show it in 2D |
|---|---|---|---|---|
| Nine speeds, up to 120 days a second (`,` / `.`) | live.rs | same (groups 2–3): nine speeds, keys `1`–`9`, as in the 3D client | | |
| Day and night: the sun, the moon and the town lights show the hour, and people go home to rest | env.rs, sky.rs, lights.rs, crowd.rs | same (groups 2–3): a dark veil from dusk to dawn (with an hour of twilight) at speeds up to 12 s a day, and resting people blue-grey | | |

### Weather, seasons and the land

| Feature | 3D file(s) | 2D status | Effort | How to show it in 2D |
|---|---|---|---|---|
| Weather on the map: rain, snow, clouds and wind | env.rs, precip.rs, sky.rs | same (groups 2–3): a weather line in the HUD for the economy panel's town, and rain or snow over the view by the weather of the town nearest its centre, slanted by the wind | | |
| Ground state: snow cover, wetness and autumn colour | env.rs, terrain.rs | same (groups 2–3): the terrain image is tinted by the season, and each cell by its nearest town's snow cover and wet ground (seasons.rs) | | |
| Fields through the farming year, and how much of each town's harvest is in (`TownView::crops` against the year's peak) | fields.rs | same (groups 2–3): field and flax sites go ploughed, green, gold and stubble, reaped nearest the town first; `T` shows the harvest in, and a gold arc around the town shows it from late summer | | |
| Ice on rivers in deep winter | water.rs | same (groups 2–3): rivers and fords turn pale as the nearest town's day falls below freezing; the HUD says when they are frozen | | |
| Weather and season set by hand for a look (`Y` / `U`, `--weather`, `--season`) | env.rs | missing | S | Low value for testing. Leave it out, or add the same flags. |

### Map

| Feature | 3D file(s) | 2D status | Effort | How to show it in 2D |
|---|---|---|---|---|
| Roads between towns (`Terrain::path`) | roads.rs | same (group 1): road cells are brown in the terrain image (terrain_view.rs) | | |
| Caravans driving along the roads, more of them for more trade | roads.rs, overlay.rs | same (group 5): caravans move as dots along the flow arrows, from one on the smallest route to seven on the biggest (economy_ui.rs) | | |
| Rings around the buildings a Why? finding names | overlay.rs (`draw_why`), ui.rs | same (groups 2–3): with `Y` open, those buildings pulse in red | | |
| "Look at" a town or a person moves the camera there | ui.rs, inspect.rs, camera.rs | same (group 1): `Tab` centres the camera on the next town; `F` goes to the selected person | | |
| Follow the selected person with the camera | inspect.rs | same (group 1): `F` follows them, and panning lets go | | |
| Hide all panels to look at the world (`H`) | ui.rs | same (group 5): `F2`, since `H` is history; every panel comes back as it was (hud.rs) | | |

### People

| Feature | 3D file(s) | 2D status | Effort | How to show it in 2D |
|---|---|---|---|---|
| Colour by what a person does today: the domain worked at, soldiers dark red, idle light, resting blue-grey, children small and pale | crowd.rs | same (group 1): coloured by today's work and activity; `K` switches to the best skill domain, and the legend follows | | |
| Household members as buttons that select them | ui.rs (`person_window`) | partial: a line to the partner, members listed in `I` as text | S | Keys in `I` to step to partner, parents and children, or click the listed id. |
| Guild and rank in the person panel | missing in 3D too | 2D has it (inspector) | S | See "What the 2D client shows that the 3D one lacks". |

### Government and orders

| Feature | 3D file(s) | 2D status | Effort | How to show it in 2D |
|---|---|---|---|---|
| Govern any selected town, not only the player's | ui.rs | partial: the policy panel governs only `PlayerTown` | S | Let the policy panel follow the economy panel's town, perhaps behind a sandbox flag. |
| Commission a building and let the town choose the site | ui.rs | partial: 2D places it on the map by click | S | Add a "Commission" button beside "Place". |
| Sent orders with their dates, rejected orders in red, and commissions waiting | ui.rs | same (group 1): the policy panel lists the last four orders sent, each waiting, applied or rejected (in red); commissions waiting were already shown | | |

### War

| Feature | 3D file(s) | 2D status | Effort | How to show it in 2D |
|---|---|---|---|---|
| **Live battles** for the player's town: the simulation holds its clock while the battle runs in real time | battles.rs, live.rs (`war::order_live_battle`) | same (group 4): the 2D sim loop holds the economy while the battle waits and steps it 6 battle seconds a second (unless paused), through the sim's own calls (live_battle.rs); the squads are drawn live with battles.rs | | |
| Squad orders in a live battle (own judgment, hold, withdraw, or attack or shoot at a named enemy squad), fighters left, squad state, "Look at the field", "Fight it out" | battles.rs (`battle_panel`) | same (group 4): a battle panel at the top, one row per squad with its fighters left, state and morale, and order buttons for the player's fighting squads | | |
| Live battles on the soldier model (`battle.live_model`): each figure stands where its soldier stands, faces their way, lies where they fell, greys when they run | battles.rs (`pose_soldiers`) | same: zoomed in (`SOLDIERS_BELOW`), every soldier is a dot where they stand, the fallen a small cross, runners grey and blinking; zoomed out, a block per squad at its soldiers' centre (live_battle.rs, battles.rs `draw_soldiers`) | | |
| Armies on the move: soldiers walk to their squad's goal in dark red, captives apart | crowd.rs | partial (group 1): soldiers are red and captives grey; no flag at the squad's goal yet | S | Draw a squad's goal as a small flag. |
| Battle replays beside the defending town | battles.rs | same (blocks per squad, 2D battles.rs) | | |

### Town information

| Feature | 3D file(s) | 2D status | Effort | How to show it in 2D |
|---|---|---|---|---|
| Town panel: population, treasury, ruler, buildings by type, market stock and price | ui.rs | same (group 1): `T` lists the buildings by type (n × type); the HUD shows the ruler, the economy panel the goods | | |
| Money in the reports: treasury, mean wealth, bread price | ui.rs (Town, History) | same (group 1): `T` shows treasury and mean wealth, `H` treasury, mean wealth and bread price | | |
| History as line charts with seven series, the selected town in bold | ui.rs (`chart`) | same (group 5): `H` shows the seven series as line charts, one line per town in its colour over a shared range, the economy panel's town in bold and on top; gizmos drawn by a second camera above the panels (reports.rs) | | |
| Weather details: temperature, sky, wind and ground | ui.rs | same (`T`) | | |
| Food security and needs | ui.rs | same (`T`) | | |
| Product space per town | ui.rs, overlay.rs | partial: 2D shows it for the selected person or the population | S | Add a town mode that follows `Tab`. |

### Questions to the view

| Feature | 3D file(s) | 2D status | Effort | How to show it in 2D |
|---|---|---|---|---|
| Why? as a foldable tree, asked after every tick | ui.rs, live.rs | same (`Y`, an indented text tree) | | |
| Person lookup on click (`sim_core::view::person`) | inspect.rs | same (`I`) | | |
| Event log | ui.rs | same (`L`) | | |

## What the 2D client shows that the 3D one lacks

| Feature | 2D file | Note for the 3D client |
|---|---|---|
| Economy panel: money flows (minted, issued, taxed, spent), who works at what, shortages | economy_ui.rs | 3D shows only stock and price. |
| Money rings per town on the map | economy_ui.rs | Could be an overlay stop. |
| Match goal and its progress | economy_ui.rs, hud.rs | Missing in 3D. |
| More levers: subsidies per product, schools (domain and seats), profit tax, procurement, charters, army share | policy_ui.rs | 3D has taxes, wages, spending, immigration, minting, war, raids and commissions. |
| Placing a building on the map | policy_ui.rs | 3D commissions only. |
| Agent inspector: skills, mind, culture fit, physique, guild and rank | hud.rs | 3D shows needs, larder, goods, gear and household, but no skills or guild. |
| Resource level of each nature site, as a ring | view.rs | Missing in 3D. |
| World summary: complexity (ECI) per town, eras, culture table | hud.rs | Missing in 3D. |
| Quicksave and quickload (`F5` / `F9`) | saving.rs | Missing in 3D. |

## Recommended order

1. **Quick wins for testing (S), done (group 1):**
   - money in the `T` and `H` reports;
   - people coloured by activity, with soldiers in red;
   - the camera follows `Tab` and the selected person;
   - roads;
   - the sent-orders list;
   - building counts in `T`.
2. **Seasons and harvest (S–M), done (groups 2–3):**
   - season and snow tint;
   - field crop stages and harvest %;
   - river ice;
   - a weather line and rain or snow over the map.

   Farming and fuel follow these.
3. **Speeds 6–9 and Why? rings (S), done (groups 2–3).**
4. **Live battles (L, then M), done (group 4):**
   - the clock hold in the 2D loop;
   - then the battle panel with squad orders.

   This is the largest gap and the only one that changes how the 2D client
   runs the simulation.
5. **Line charts for history (M), caravans moving along the arrows (S), and
   the hide-UI and follow toggles (S), done (group 1 for follow, group 5 for
   the rest).**
6. **The other direction:** bring the 2D-only levers, economy panel, skills
   inspector and quicksave to the 3D client. That is a separate package.

## The 2D reports panel

One panel in the middle of the screen shows one report at a time. Its key
opens it, and pressing the same key again closes it. Reports follow the
economy panel's town (`Tab`) and the selected person. Each report is computed
once per simulated day (or when its subject changes), not every frame.

| Key | Report |
| --- | --- |
| `Y` | Why? for the town |
| `T` | Town: weather and season, people and money, buildings by type, food security, needs, armory |
| `H` | History as line charts: population, treasury, food per resident, mean wealth, bread price, soldiers, buildings, one line per town |
| `L` | Event log, newest first |
| `I` | Person: needs, larder, household goods, equipment, household (opens itself when you click someone) |
