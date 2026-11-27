# Step 20: tilt-shift

Photographers make real towns look like models with a tilt-shift lens. The
lens's plane of focus is tilted, so only a band across the picture is sharp.
Our eyes know that shallow focus from looking at small things up close, so
the town reads as a miniature. Here the same look is drawn after the bloom.

## How (`src/tiltshift.rs`, `assets/shaders/tiltshift.wesl`)

For every camera with `TiltShift`, after the bloom and before tonemapping
(the image is still HDR):

1. **Downsample:** the image is shrunk to half, a quarter and an eighth of
   its size. Each step is the bloom's 13-tap average (step 08), which is
   already a blur.
2. **Composite:** each pixel's distance from the sharp band (`centre ± band`,
   in screen height) sets how much blur it gets (`a`, 0 to 1 over `fade`).
   It fades from the sharp image into the quarter-size mip, then further out
   into the eighth-size mip. Both are read through a 3×3 tent filter so
   their texels don't show.

Since the blurred mips are a quarter and an eighth of the screen's size, a
blur that spans dozens of pixels costs only a few texture reads per pixel.

| setting | default | meaning |
| --- | --- | --- |
| `centre` | 0.55 | where the sharp band sits (0 top, 1 bottom); a little below the middle, where the camera looks |
| `band` | 0.22 | half its height |
| `fade` | 0.5 | how far beyond the band the blur becomes full |
| `strength` | 0.5 | how far the blur goes: at 0.5 the edges are half blurred, from the quarter-size mip only; `X` toggles between 0.5 and 0 |

## Cost

Three small downsample passes and one full-screen composite, about the
bloom's cost.

## Exercises

- Set the band's centre from the depth under the screen's middle, so it
  follows what the camera looks at on a slope.
- Push the saturation up a little away from the band; miniature photos
  often have that.
