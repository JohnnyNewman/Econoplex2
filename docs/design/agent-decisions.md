# Agent decisions: how they work today, and a more principled replacement

Status: **agreed direction** (2026-11-09). Nils's decisions are under "Decisions"
at the end; implementation follows the migration plan in (d).

## Summary

Today an idle agent picks a job by scoring a handful of random job openings with a
hand-built feature vector (value, skill, effort, novelty, social, need). It weights
the features with a matrix of magic constants, samples by softmax, and then takes the
job if it is still open, in agent-index order. It works, and the economy is believable,
but it has three problems:

- The numbers have no economic meaning. "Value 1.2 + ambition × 1.0" can't be checked
  against anything.
- Different decisions (work, rest, learning, moving, eating) use unrelated rules, so
  they can't be traded off against each other.
- First-come picking means agent 0 gets first choice every day.

The proposal replaces this with one principle:

- **Each agent maximises expected utility over a short horizon.**
  - Utility comes from explicit needs (food, rest, safety, standing), each with
    diminishing returns.
  - Expected income is wage × the agent's own productivity, both read from the
    simulation (capability vectors, prices).
  - Learning is an investment that pays off later, discounted.
- **Choice is bounded-rational:** a logit, whose temperature and discount rate come
  from the agent's mind vector.
- **Jobs clear in a matching step,** not first-come.

The new model would arrive behind a switch in `models.ron` (`decision.model: Features |
Utility`), in small PRs, with tests comparing the two.

---

## (a) How decisions work today

### The daily loop

```mermaid
flowchart TD
  sense[sense: hunger, fatigue, eat from the town market] --> decide
  decide{idle adult?} -- yes --> score[score up to 6 random open jobs + habitual job + rest]
  score --> pick[softmax pick, in agent-index order; job taken if still open]
  pick --> assign[assign: group pickers into teams, start tasks]
  assign --> produce[produce: work, success, quality, wages]
  produce --> learn[learn: practice, master teaches team]
  learn --> other[socialize, body, market, organize, military, migrate, life]
  decide -- "no (working, soldier, child)" --> other
```

### Job choice (`systems/decide.rs`)

For each idle adult:

1. **Candidates:** `candidates = 6` random jobs from the town's open jobs, plus the
   agent's last recipe if still open, plus **rest**. A job is open when its inputs,
   nature and building slots allow at least one more worker.
2. **Features** of each job (one row each):

   | Feature | Formula | What it stands for |
   | --- | --- | --- |
   | value | `tanh((profit_per_day − town mean) / value_scale) × σ(6·margin)` | money, if you can do it |
   | skill | `σ(6·margin) − 0.5` | fit: chance of success, centred |
   | effort | `−(0.3 + physical load) × (0.5 + fatigue)` | tiredness cost |
   | novelty | `(1 − proficiency/mastery) × (margin > −0.3 ? 1 : 0.2)` | room to learn |
   | social | 1 if in the domain's guild, `charter_pull` if chartered | belonging |
   | need | `food_out × 0.25 × food scarcity × (0.5 + hunger)` | feeding the town |

   Here `margin = (capability · skill + physique bonus) × state factor − difficulty`.
   Rest has its own feature row: `[0, 0, 1.5·fatigue, 0, 0, fatigue]`.
3. **Weights:** `w = bias + mind_weights · mind + state_weights · state`, a 6×8 and a
   6×4 matrix of hand-set constants (`models.ron`, `decision`).
4. **Pick:** softmax over `w · features` with `temperature = 0.25`, using the agent's
   own random stream.

Since #20, every agent scores every open job in parallel. The picks then happen in
agent-index order. A job that fills up drops out for everyone later in line.

### Other decisions, each with its own rule

| Decision | Rule today | Where |
| --- | --- | --- |
| Eat | if hunger > `eat_threshold`, eat the best food in stock, pay the price if you can | `needs.rs` |
| Rest | an option in job choice; resting recovers fatigue | `decide.rs`, `needs.rs` |
| Learn | never chosen: a side effect of working (practice) and being taught | `work.rs` |
| Join a guild / buy a tool | proficiency over a threshold; buy the best affordable tool, on credit | `organize.rs` |
| Migrate | 5% of idle adults a month compare towns: food, own trade's price, outlook, danger, distance, trusted people; move if gain > `min_gain` | `migrate.rs` |
| Have children | food foresight and age, not a choice | `life.rs` |
| Soldier | recruited by the town, not a choice | `military.rs` |

### Known artefacts

- **Magic constants with no units:**
  - 66 weights in two matrices, plus `value_scale`, `temperature`, and the 0.25,
    0.3, 6.0 and 1.5 inside the features.
  - Tuning one changes everything, and none can be checked against data.
- **Value is relative to the town mean,** squashed by `tanh`. An agent can't tell a
  job that pays 3× from one that pays 30×.
- **Money is not utility:**
  - Wealth never enters the job choice. A rich agent works as hard as a poor one.
  - Savings only matter through the wealth tax.
- **Learning is accidental:** "novelty" rewards unfamiliar jobs, but nothing
  compares the cost of being bad at a job now with the pay of being good at it later.
  Schools and guilds do the real work.
- **First-come picking:**
  - Agent 0 picks first every day.
  - Popular jobs fill with whoever happens to be early, not with the best-suited
    workers.
  - Wages don't adjust to how many people want a job; only capacity limits it.
- **Separate rules can't trade off:** an agent can't decide to move to a richer town
  *instead of* retraining, or to rest because it is rich.
- **Temperature is global:** every agent is equally noisy. The mind vector only tilts
  the weights.

---

## (b) A principled replacement

### Utility from needs

Each agent `i` has needs `k ∈ {food, rest, safety, standing}` with satisfaction
levels `s_k ∈ [0, 1]` derived from its state:

- food = 1 − hunger
- rest = 1 − fatigue
- safety = health, and no raid this year in its town
- standing = wealth relative to the town's median, plus guild role

Utility is a weighted sum with **diminishing marginal utility** (constant relative
risk aversion):

```text
u(s) = Σ_k a_k · (s_k^(1−ρ) − 1) / (1 − ρ)        (ρ > 0; log at ρ = 1)
```

- `a_k` are need weights from the mind vector. For example, ambition raises
  standing, and sociability raises standing through guild membership.
- `ρ` sets how strongly a short need dominates. A starving agent cares about little
  but food.

Money matters only through what it buys: next period's food (at the town's price),
rest (not working), and standing. That puts wealth into every decision, consistently.

```mermaid
flowchart LR
  subgraph state[Agent state]
    h[hunger] --> food
    f[fatigue] --> rest
    hl[health, raids] --> safety
    w[wealth vs median, guild role] --> standing
  end
  subgraph utility[Utility, diminishing returns]
    food --> U
    rest --> U
    safety --> U
    standing --> U
  end
  mind[mind vector] -- need weights a_k, temperature, discount --> U
  U[u = sum a_k s_k^1-rho / 1-rho]
```

### Expected value of an action over a short horizon

An action is: work at job `j`, rest, train (study or apprentice), or move to town
`m`. Its value is the expected utility over a horizon `H` of a few weeks, discounted
by `β` per day:

```text
V(action) = Σ_{d=0..H} β^d · E[ u(s_{d+1}) | action ]
```

with a simple forecast of the state:

- **Expected income from job j:**
  `E[pay] = p_success(i, j) × wage_j × output_j`.
  - `p_success` comes from the capability vector (the sigmoid already used in
    `produce`).
  - `wage_j` is the agent's *expectation* of pay per unit of value, learned from
    experience (below).
  - `output_j` is the recipe's output at the agent's *expected* price.
- **Effort:** working adds fatigue `(0.5 + physical load)`, so the rest need falls,
  and its marginal utility rises with fatigue.
- **Learning as investment:**
  - Working at `j`, or training, raises the capability along `j`'s skill by the
    known practice rule.
  - The value of that is the discounted extra income it brings in the jobs the agent
    expects to do later: `Σ β^d × Δp_success × wage × output`, for the best of those
    jobs.
  - This replaces "novelty". Young agents (long horizon, low `β` discounting) invest;
    old agents don't.
- **Moving:**
  - The same income forecast with the destination's prices and food, minus a one-off
    utility cost for the road and leaving ties.
  - This replaces migration's separate weights, and lets an agent compare moving with
    retraining.
- **Eating:** stays mechanical (hunger above a threshold), but buying food now
  competes with saving for standing. A poor agent eats; a rich one also eats, because
  food utility dominates when hungry.

**Adaptive expectations:** each agent keeps smoothed expectations of the prices and
pay it has seen (exponential moving averages: `E_t+1 = E_t + λ (observed − E_t)`, with
`λ` from curiosity). Agents react to market signals with a lag, and agents in
different towns can disagree. That is a source of realistic mistakes.

### Bounded rationality: logit choice from the mind

```text
P(action) ∝ exp(V(action) / τ_i),     τ_i = τ_0 × (1 + risk − diligence)
```

- Diligent agents choose sharply; risk-takers explore.
- Discount `β_i` comes from ambition (patient, long-horizon agents invest in skills).
- The 66 constants shrink to a handful with meaning:
  - need weights per mind axis
  - `ρ`
  - `τ_0`
  - `H`
  - `β` range
  - `λ`

### Job matching instead of first-come

```mermaid
flowchart LR
  subgraph phase1[Parallel: every idle agent]
    V[value of each open job and of rest / train / move] --> pref[ranked list or logit draws]
  end
  subgraph phase2[Matching per town and job]
    pref --> m{more applicants than places?}
    m -- no --> take[everyone gets the job]
    m -- yes --> rank[employer ranks by expected output per wage]
    rank --> take2[best-suited get it; others fall to their next choice]
  end
  take --> teams[teams form, as today]
  take2 --> teams
  phase2 --> signal[unfilled places and queues adjust next day's wage expectations]
```

- **Deferred acceptance** (Gale–Shapley) between agents (by their logit-sampled
  preference order) and job openings (by applicants' expected output). Results don't
  depend on agent index, and the best-suited workers get scarce places.
- **Wage signal:** queues for a job and unfilled places feed back into everyone's
  expected wage for it. That gives a soft labour-market clearing, and nobody sets a
  wage directly.

---

## (c) What changes, what stays, and what it costs

**Stays:**
- capability vectors, practice and teaching
- recipes, prices and the money ledger
- teams, trust and guilds
- every player and AI lever (they act on prices, pay and demand, which now feed
  expected utility directly)

**Changes in emergent behaviour (expected):**
- **Wealthier agents rest more and work less;** poor and hungry ones take any work.
  Income inequality now affects labour supply.
- **Young agents invest in skills** where future pay is high. Old agents stay put.
  Schools and charters still matter, now as cheaper or faster learning.
- **Migration and retraining compete:** depressed towns lose young skilled workers
  first.
- **Better matching:** a scarce smithy slot goes to the best smith, not the first in
  line. Production quality should rise somewhat. It's not guaranteed: if expectations
  are wrong, matching can be worse.
- **Wage and price lag:** agents chase last month's prices, so cobweb cycles become
  possible. We'll need to watch for oscillation, as with the AI governors.

**Determinism and parallelism:**
- Values for all idle agents are computed in parallel from a snapshot, with each
  agent's counter-based random stream, as today.
- Matching is a deterministic algorithm over sorted lists. It is per town and job, so
  towns can run in parallel.
- Same seed gives the same hash, whatever the thread count, as now.

**Performance at 50k agents (estimate):**
- **Today:** decide takes about 8–9 ms per tick, roughly 3 ms scoring and 5 ms
  picking.
- **Valuing actions:**
  - About 50 open jobs × a short horizon. With the horizon solved in closed form (a
    geometric series), each job costs about as much as today's feature row.
  - Plus about 4 non-job actions.
  - So roughly 3–5 ms in parallel.
- **Deferred acceptance:** O(applications) per town, about 30k agents × a few
  proposals. Roughly 2–4 ms single-threaded, less per town in parallel.
- **Expectations:** a small array per agent (prices of goods it cares about). Memory
  is about 50k × 47 floats ≈ 9 MB; cap it to the goods of the agent's town if needed.
- **Net: likely similar to today,** about 6–10 ms. I'll measure at each PR.

---

## Module boundaries

Decision-making is built from separate components, each behind a trait in
`sim_core::decision`. Each is chosen in `models.ron`, the same way the existing models
(`team_pooling`, `learning`, `diffusion`, ...) are today, and each has its own params
block and its own unit tests. A component sees only its inputs below. It doesn't
reach into the agent store or another component's internals. The decide, matching
and wage systems wire them together.

```mermaid
flowchart LR
  subgraph inputs[Read-only inputs]
    obs[market observations: prices, realised pay per town and job]
    ag[agent view: needs state, mind, capability along a skill, wealth, age]
    job[job offer: recipe, output, difficulty, physical load, regular or apprentice]
  end
  E[Expectations<br/>forecast price and pay]
  U[Utility<br/>needs to value]
  L[LearningValue<br/>value of skill gained]
  C[ChoiceRule<br/>argmax or logit]
  W[WageAdjustment<br/>queues to pay factor]
  M[JobMatching<br/>places, apprentices]
  G[Migration<br/>value of moving]
  obs --> E
  ag --> E
  E -- expected pay, price --> U
  ag --> U
  job --> U
  ag --> L
  job --> L
  E --> L
  U -- value per action --> C
  L -- investment value --> C
  E --> G
  U --> G
  G -- value of moving --> C
  C -- ranked applications --> M
  M -- matches, queues, unfilled places --> W
  W -- pay factor --> obs
  M -- jobs taken --> out[assign: teams and tasks]
```

| Component | Trait input | Trait output | Implementations |
| --- | --- | --- | --- |
| Expectations | the agent's last expectation, an observed price or pay, the agent's curiosity | the new expectation | `Fixed` (use the current price), `Adaptive` (EMA with rate from curiosity) |
| Utility | needs state, need weights from the mind, a planned change to the needs (food bought, fatigue added, standing gained) | utility value | `Crra { rho }` |
| LearningValue | skill gain along a skill, expected pay of jobs using it, horizon, discount | present value | `None`, `Discounted { horizon_days, discount }` |
| ChoiceRule | values per option, the agent's temperature, a random stream | chosen option, or a ranked list for matching | `Argmax`, `Logit { base_temperature }` |
| WageAdjustment | applicants, places, current factor | new factor | `Fixed`, `ExcessDemand { step, min, max }` |
| JobMatching | ranked applications per agent, places per job, apprentice places | matches | `FirstCome` (today's), `DeferredAcceptance { apprentice_wage, apprentice_share_min, apprentice_share_max }` |
| Migration | the value of staying vs. moving to each town | value of each move | `Weighted` (today's migrate rule), `Utility` |

**Today's model stays as one implementation:**
- `decision.model: Features` keeps the current feature-matrix scoring, with
  `JobMatching::FirstCome`, `WageAdjustment::Fixed` and `Migration::Weighted`.
- `decision.model: Utility` composes the new components.
- Components can be mixed: for example `Features` scoring with `DeferredAcceptance`
  matching, to test one change at a time.

All seven implementation PRs keep this structure: each adds or changes one component
(plus its params block and tests) and the wiring that uses it.

## (d) Migration plan

Everything is behind `decision.model: Features | Utility` in `models.ron`. `Features`
stays the default until `Utility` wins on the comparisons below.

1. **Expectations:** per-agent price and pay expectations (EMA). They are recorded in
   both models but not yet used by `Features`. Test: expectations converge to steady
   prices.
2. **Utility function and needs:** `u(s)` with need weights and `ρ` from the mind,
   pure functions with unit tests (monotone, concave, starving dominates).
3. **Utility job choice:** value = expected income utility + effort, logit with the
   mind temperature, still picked first-come. Tests:
   - side by side with `Features` over 40 years on 3 seeds
   - population, hunger, ECI, eras and output within agreed bands
   - wealthier agents work fewer days
4. **Learning as investment:** the discounted skill-gain term. Test: young agents take
   more training jobs than old ones, and schools still beat no school.
5. **Matching:** deferred acceptance replaces first-come in both models. Tests:
   - order-independence: shuffling agent indices gives the same set of matches, up
     to equal-score ties
   - scarce places go to higher expected output
6. **Moving as an action:** fold migration into the same valuation (behind its own
   switch). Test: migrants still follow demand; closed borders still keep them out.
7. **Switch the default** to `Utility` if the comparisons hold, and remove the old
   matrices a release later.

Each step is a small PR with its own tests and a before/after table, merged when CI is
green.

---

## Decisions (2026-11-09)

1. **Horizon: weeks.** Agents plan a few weeks ahead (`H` ≈ 30 days), plus the
   discounted skill-investment term for learning.
2. **Hiring: apprentice places, the rest by expected output.**
   - Workplaces (a town's buildings of one kind, or the guild of their domain) offer
     **apprentice places** besides their regular places.
   - Apprentices are paid less: an `apprentice_wage` factor in `models.ron`, e.g. 0.4
     of the regular pay for the job. They also learn faster from the masters they
     work beside (teaching already exists).
   - **How many apprentice places is the workplace's own economic choice,** within
     `apprentice_share_min`..`apprentice_share_max` from `models.ron`. Each month it
     compares:
     - **cheap labour now:** an apprentice's expected output × price − the
       apprentice wage, against a regular worker's output − the regular wage
     - **a skilled worker later:** the discounted extra output once trained, counted
       only for the share expected to stay in the town (trust and guild ties raise
       it)
     - It adds apprentice places while that sum is positive and the town is short of
       skilled hands in the domain, and removes them otherwise.
   - Applicants say whether they apply as apprentice or regular worker. That is
     their own utility choice: lower pay now, faster skill growth, better pay later.
     It falls out of the learning-as-investment term.
   - **Regular places** go to the applicants with the highest expected output;
     **apprentice places** go to the applicants with the most to gain (lowest current
     skill, highest aptitude), so beginners get a way in.
3. **Pay follows market forces.**
   - Each job's pay is no longer a fixed share of value added.
   - A per-town, per-recipe **wage factor** multiplies the town's base wage share.
     Every day it moves by a small step against the excess demand for regular places
     ((applicants − places) / places), clamped to a range in `models.ron`.
   - A job with a queue pays less. A job with unfilled places pays more, up to its
     value added.
   - Apprentice pay is `apprentice_wage` × that job's current pay, so it follows the
     market too.
   - The player's and governors' `wages` lever sets the **base** share; the market
     factor rides on top.
   - Agents' expected pay learns from realised pay: queue → wage → expectation →
     choice.

The migration plan changes accordingly:

- **5. Flexible pay:** wage factor per town and job. Tests:
  - a queued job's pay falls, an understaffed job's pay rises
  - the ledger still balances
  - pay stays in range
- **6. Matching with apprentice places:**
  - deferred acceptance for regular places by expected output
  - apprentice places by gain, at `apprentice_wage`
  - the workplace's monthly choice of its apprentice share
  - Tests:
    - order-independence
    - scarce regular places go to higher expected output
    - apprentice share stays within its bounds and grows when trained workers are
      short and worth more
    - beginners get into trades they couldn't enter by output alone
