# Slug Font Rendering

GPU-accelerated vector text rendering via direct bezier curve evaluation in fragment shaders. Resolution-independent, perfect at any zoom/scale, no glyph atlas needed.

## Why This Matters

The current iced text rendering stack (`cosmic-text` + `cryoglyph`) is atlas-based and produces slightly off results. Slug is a fundamentally different approach — curves are evaluated per-pixel on the GPU, which means:

- No rasterization artifacts at any scale factor
- No atlas memory overhead
- Clean rendering at fractional scale factors (relevant for our DPI auto-detection)
- Correct subpixel positioning without hinting hacks

## Status

The Slug algorithm patent (US #10,373,352) was **dedicated to the public domain** on March 17, 2026 by Eric Lengyel. Reference shaders are available under MIT license.

- Blog post: https://terathon.com/blog/decade-slug.html
- Reference implementation (vertex + pixel shaders, MIT): https://github.com/EricLengyel/Slug
- Original JCGT paper: "GPU-Centered Font Rendering Directly from Glyph Outlines" (2017)

## Actionability

Not actionable now. This would be a change to the iced rendering layer, not our app code. Possible paths:

1. **Upstream iced adoption** — if iced replaces its text renderer with a Slug-based approach, we get it for free
2. **Custom iced fork patch** — if text rendering quality becomes a blocker, we could prototype a Slug-based text renderer for iced's wgpu backend
3. **Standalone investigation** — evaluate the reference shaders against our use case (email client = lots of text at various sizes, mixed fonts)

Revisit when text rendering quality becomes a priority or when someone in the iced/wgpu ecosystem picks this up.
