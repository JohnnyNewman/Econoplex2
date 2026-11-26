# Step 18: colour grading

The light already follows the sun and the weather (sky.rs), and the ground
follows the season (`seasonal` in fog.wesl). A colour grade on top leans
each moment towards its own mood, the way a painter would:

| moment | grade |
| --- | --- |
| golden hour (low sun above the horizon) | warmer, a little richer and more contrasty |
| night | cooler, much less colour |
| autumn | warm and saturated |
| winter | cool and grey |
| spring | a touch greener and fresher |
| overcast, rain | flatter and duller; clouds also take most of the golden hour away |

## How (`src/grading.rs`)

It's Bevy's own `ColorGrading` component on the world camera. Bevy applies
it in the tonemapping pass, which runs anyway, so there's no extra pass and
no material changes. Each frame, `Mood::of` reads the sun's height, the
season weights (`Environment::spring`, `autumn`, `winter`) and the weather.
`grade` turns those into a white balance (temperature and tint) and a
saturation and contrast. The component is written only when the grade
changes.

The values are small. Bevy's temperature moves the colours a long way: a
few hundredths are plenty, and the first try at 0.1 turned a summer evening
orange. Cooling alone only moves the CIE x coordinate, which makes the
image mint, so a little magenta (tint) goes with it.

## Not yet: tilt-shift

A blur at the top and bottom of the screen makes the world look like a
model (the "miniature" look). That needs its own full-screen pass with a
depth-aware blur, so it's left for later.

## Exercises

- Give a burning raid (once the sim has them) a red cast.
- Lift the shadows a little at noon in summer (`shadows.lift`), for a hazy
  heat.
