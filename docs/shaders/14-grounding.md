# Step 14: grounding

Houses, buildings and trees looked pasted onto the ground wherever the sun's
shadow didn't fall: on the sunny side, at dusk, under an overcast sky. Real
objects darken the ground right at their feet and are darker there
themselves, because the ground and the object hide part of the sky from each
other (ambient occlusion). Two cheap fakes give that look.

## 1. Baked into the objects (vertex colours)

When a building, a town's houses or a forest chunk is baked into one mesh
(`bake.rs`), `Baked::occlude_from` darkens each vertex by `bake::occlusion`:

- **Wall feet.** From 55 % brightness at the object's ground to full at
  0.6 units above it (smoothstep).
- **Undersides.** Faces that look down (eaves, the underside of a crown) get
  another 0.62.

It runs once, when the mesh is baked, and costs nothing per frame. Walls only
have vertices at their top and bottom, so the gradient runs over the whole
wall height on low walls.

## 2. On the ground (the contact map)

`grounding.rs` keeps a greyscale texture over the whole terrain, the contact
map (1024 × 1024, about 0.2 units per texel). Each texel holds how hemmed in
that spot of ground is: 1 under a footprint, fading smoothly to 0 at
`REACH` (2.4) times the footprint's radius. Footprints are discs:

| What | Radius |
| --- | --- |
| tree | 0.5 × its scale |
| house | 0.8 |
| workplace | 2.4 |

The CPU draws the discs (max-combined) at start and again whenever a town's
houses or buildings change (`Footprints::dirty`, set by `live.rs`), about once
a game day. Only the texture's pixels are uploaded, and the terrain material
is touched once so Bevy rebuilds its bind group around the new texture.

The terrain shader (`terrain.wesl`, bindings 104/105) samples it at the
fragment's x, z (`Terrain::contact` holds where the map lies) and darkens the
ground's colour by up to 55 %, before lighting, so it works in sun, shade and
at night alike.

## Cost

One texture sample per terrain fragment. The CPU redraw takes a few
milliseconds and runs about once a game day.

## Ideas for later

- Footprints in the shape of the building (rectangles) instead of discs.
- Screen-space ambient occlusion (SSAO) would catch everything, people and
  hills included, but costs a full-screen pass every frame.
