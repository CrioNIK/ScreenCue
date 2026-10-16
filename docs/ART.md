# Visual assets

The ScreenCue mark was generated with the built-in OpenAI image generation tool for this project. No external brand artwork is included. The PNG and ICO are source/documentation assets; the Rust executable loads neither. The overlay uses thin code-rendered shapes and compact text, with no large branded header or raster artwork.

The arrow and empty target contour are independent drawable objects. The target can be an ellipse, rectangle, rounded rectangle, or polygon; it has no fill. The arrow ends before the target edge with a 12 px gap. The default automatic color is a local contrast and hue-rarity heuristic using one transient patch of at most 128 × 128 pixels near the target. It does not save, transmit, or provide that patch as a screenshot. An explicit `#RRGGBB` color bypasses sampling. The color is chosen once per hint, with no guarantee of contrast on every background.

Prompt used:

> Use case: logo-brand. Asset type: small app identity sticker for ScreenCue, a tiny desktop screen hint utility for a punk user. Create one compact emblem, square composition, true transparent background. A bold angular upward arrow with a slightly hand-cut silhouette points toward an empty ring but STOPS SHORT of the ring with a clear gap; the ring interior stays empty. Punk zine / DIY screenprint attitude, subtle rough ink edges but very simple broad shapes legible at tiny icon sizes. High contrast acid yellow and warm off-white strokes with a small hot-pink offset accent, restrained charcoal outline. No words, letters, numbers, slogans, mockup surfaces, gradients, shadows, or extra symbols. Emblem fills most of the square with modest transparent padding. Crisp graphic raster artwork, not a photo.

`preview.png` is a deterministic render of the actual Rust overlay drawing code, not a capture of a user's desktop. Reproduce it with `screencue --render-preview docs/preview.png`. Preview generation uses an explicit color and does not read desktop pixels. On macOS, automatic color selection uses an agent-supplied `background_colors` palette; without a palette it reports a fallback color.
