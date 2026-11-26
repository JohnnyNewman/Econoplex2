# Step 17: cloud shadows

Under broken cloud the land used to be lit evenly, only a little dimmer.
Now the shadows of the clouds drift over it: terrain, grass, buildings,
people and water all darken where a cloud passes between them and the sun.

## How (`cloud_shadow` in `assets/shaders/fog.wesl`)

The clouds are a flat deck of fractal noise (three octaves of
`season_noise`), 40 units above the ground, 26 units per noise cell. For a
point `p`, follow the ray toward the sun up to the deck:

    deck = p.xz + sun.xz / sun.y · (40 − p.y)

and look up the noise there. The wind moves the deck, 0.5 to 3 units a
second (`fog.wind`: its direction and strength). As in the sky (step 11),
more cover means a lower threshold, so more of the noise counts as cloud.
Under a cloud the point keeps 30 % of the sun.

The result is a new field of the shared toon lighting, `ToonParams::sky`
(1 clear .. 0.3 shaded), which scales the sunlit part and the rim light in
`toon_terms`. So every material that uses the toon lighting gets the shadows
the same way: each sets `params.sky = cloud_shadow(position, env.fog)`.

## Broken cloud and overcast

Patches only make sense while there are gaps. As the cover goes from 0.8 to
1 the patches fade out, and the sun dims everywhere instead
(`sky_light` in `src/sky.rs`). That global dimming now grows with the square
of the cover (`1 − 0.75 · cover²`), so a half-cloudy day isn't dimmed twice,
once by the patches and once overall.

The cloud cover reaches the shaders through the shared buffer
(`fog.debug.z`), next to the wind and the sun. No material changes.

## Limits

The clouds in the sky are drawn on the dome, not on this deck, so a shadow
doesn't sit exactly under the cloud you see. Both follow the same cover and
wind, so they read as the same weather.

## Cost

Three noise octaves per lit fragment, on every lit material.

## Exercises

- Let clouds thin out over the sea in the afternoon.
- Brighten the ambient light a little under a cloud (skylight from its edges).
