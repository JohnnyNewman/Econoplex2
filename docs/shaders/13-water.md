# Step 13: water

Rivers and the sea around the map were a flat blue plane. Now one plane at
the water level (y = -0.6, under which the terrain sinks its river beds to
-1.2) is shaded by `assets/shaders/water.wesl` (`src/water.rs`).

## What the fragment shader does

1. **Depth.** The shader reads the terrain's heightmap (the same texture the
   terrain and the crowd read), bilinear by hand because the float texture
   isn't filterable everywhere. Depth = water level - ground. Beyond the map's
   edge the plane is open sea and counts as deep. Shallow water is light and
   green, deep water dark and blue (`WaterUniform::shallow`, `deep`).
2. **Ripples.** The surface's slope comes from three travelling waves along the
   wind (`fog.wind`, from the shared buffer) plus slow noise, so the pattern
   doesn't repeat. They grow with the wind's strength and fade out with
   distance, since far ripples would only sparkle. While the ground is wet (it
   rains), little rings pop up on a grid of cells. The slope becomes the
   normal the lighting uses.
3. **Lighting.** The same toon ramp and sun shadows as everything else
   (`toon_terms`), plus the nearby lamps (`lamp_term`).
4. **Sky.** At grazing angles water reflects: the fresnel term mixes in the
   horizon colour of the fog (`fog.color`). It's no true reflection, but it
   matches the sky the eye sees there.
5. **Sun glint.** The sun reflected by the rippled normal, sharp
   (exponent 180) and brighter than white, so the bloom gives it a halo. It's
   gone at night.
6. **Flow.** A flow map (water::flow_map, one texel per heightmap corner)
   holds, for each river texel, the direction of the nearest stretch of its
   river (downstream = the order of the river's points) and how much river
   it is (fading over the banks). In the rivers the ripples are a smooth
   pattern of four sine waves pushed downstream at 1.4 units a second. A
   pattern pushed by a direction that differs from place to place would tear
   at the bends, so two copies take turns over three seconds: one drifts and
   fades out while the other starts fresh and fades in (the flow-map
   technique). Foam drifts with the river too. The sea keeps the wind.
7. **Foam.** Where the water is shallower than about 0.14 units, noise thresholds
   into drifting foam.
8. **Ice.** In deep winter (`fog.ground.w`) the water turns pale, flat and matt.
9. **Fog**, as in every material.

Everything that changes over time comes from the shared buffer
(`fog::FOG_BUFFER`), so the material never changes after it is made (see
docs/profiling.md for why that matters).

## Cost

One plane, one material, a fragment shader of four texture loads, three
waves and a lamp loop. It adds no entities and no CPU work.

## Ideas for later

- Smoother shores: the terrain's river beds follow the grid's cells, so the
  banks are blocky; a few subdivisions or a shore offset in the terrain mesh
  would round them.
- Keep grass and trees off water cells (some blades grow into the river).
- Screen-space reflections of the hills and towns, once the GPU budget allows.
