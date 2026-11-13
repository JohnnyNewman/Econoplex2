# Step 12: lights in the towns at night

Windows, lanterns and fires glow after dusk, and the lamps nearest the camera
light the walls and ground around them. None of it uses Bevy's own lights.

## Glowing parts

Baked buildings carry their colour as a vertex colour (`bake.rs`). An opaque
material ignores its alpha, so the alpha marks what lights up
(`buildings.rs`, `glow_marker`):

| alpha | part | by day | at night |
| --- | --- | --- | --- |
| 1.0 | everything else | | |
| 0.25 | window | dark glass | warm light; fewer lit as the night goes on |
| 0.5 | lantern (on a post by every third house) | dark | lit, slight flicker |
| 0.75 | fire (bakery oven, smithy forge, furnace mouth) | dim glow | bright, flickering |

`town_glow` in `lights.wesl` turns the marker into light, added to the lit
colour of `toon.wesl` brighter than white, so the bloom pass (step 08) gives
it a halo. How dark it is comes from the sun's height in the shared fog
block. A random value per house-sized cell of ground decides which windows
are lit, so a house is lit or dark as a whole.

## Light that falls on things

Each frame `lights.rs` picks the 24 lanterns and fires nearest the camera
(each building lists its own in a `Lamps` component) and writes them after
the fog in the shared buffer (`fog::FOG_BUFFER`, `struct Env` in
`lights.wesl`). The toon and terrain shaders add `lamp_term` for each: a soft
pool that fades to nothing at its reach, wrapped round so walls facing away
get a little. By day the count is 0 and the loop does nothing.

Only the buffer's contents change, so no material does (see step 04 and
docs/profiling.md for why that matters). Bevy's point lights would also
work, but each one adds CPU work to Bevy's light clustering, and our toon
lighting only reads the sun.

Cost on the laptop at night (release, alternating 20 s runs): about 0.5 to
0.8 ms per frame.
