# Agent decisions: how they work today, and a more principled replacement

Status: **proposal, for discussion**. No code changes until we agree on the direction.

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

## Decisions for Nils

1. **Horizon and foresight.** Should agents be myopic (a few weeks, cheap, more
   mistakes), or plan over years (skills, migration, savings; costlier and less
   emergent)? I suggest weeks, plus a discounted skill-investment term.
2. **How much equality of opportunity in hiring.** Matching by expected output gives
   the best-suited worker the job, which is efficient but makes beginners struggle to
   get in. Alternatives are a share of places reserved for apprentices, or ranking by
   trust and guild ties (more "who you know"). Which fits the game you want?
3. **Do wages move?**
   - Today wages are a fixed share of value added, and only prices move.
   - With matching, queues could push each job's pay up or down, a real labour
     market. That is more realistic, but it adds a second price system the player has
     to read.
   - Alternatively, queues only shift expectations and wages stay a policy lever.
