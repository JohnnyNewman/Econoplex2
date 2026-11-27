# Step 19: livestock

The sim's wool comes from pastures, nature sites that each town keeps. Now
flocks of sheep graze on them.

## What the sim reports

`TownView::pastures`: each pasture's map position and how much grass stands
on it, as a share of what it can hold. A pasture grows as a crop, so it
holds up to `crop_capacity` times its capacity (`farming.rs`).

## The flocks (`src/livestock.rs`)

A pasture carries four to nine sheep, more the more grass it has
(`flock_size`). They spread over a couple of units around it, never into a
river cell. One sheep in seven is black. A sheep is six boxes: body, head
and four legs, about 0.4 units long, a bit smaller than a person. All of a
town's sheep are baked into one mesh, rebuilt only when its pastures or
their flock sizes change.

## Grazing on the GPU (`assets/shaders/graze.wesl`)

It works like the swaying trees (step 06). Every vertex carries:

| attribute | holds |
| --- | --- |
| UV 1 | its sheep's home spot (x, z) |
| UV 0 x | 1 for the head, 0 elsewhere |
| colour | the wool or face colour |

The spot gives each sheep its own timing. The vertex shader moves the whole
sheep on a slow loop of up to half a unit around its spot (minutes per
round). It also dips the head for a few seconds at a time to graze, then
lifts it to look around. The fragment shader is the toon shader, unchanged.
The CPU does nothing per frame.

## Cost

24 vertices a box, six boxes a sheep, a few hundred sheep: tens of
thousands of vertices in one draw per town. They cast no shadows; they're
too small to matter in the sun's shadow map.

## Exercises

- Turn each sheep to face where it wanders.
- Drive the flocks home in the evening, as the people go home.
