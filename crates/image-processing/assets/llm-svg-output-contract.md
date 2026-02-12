# Output Contract

- Output must contain exactly one root `<svg>` element.
- Output must include `viewBox` with positive width/height.
- Allowed tags: `svg`, `g`, `path`, `circle`, `ellipse`, `rect`, `line`, `polyline`, `polygon`, `defs`, `linearGradient`, `radialGradient`, `stop`, `title`, `desc`, `clipPath`, `mask`.
- Forbidden tags: `script`, `foreignObject`.
- Forbidden attributes: any `on*` event handlers.
- `href` / `xlink:href` must not reference external URLs (`http:`, `https:`, `data:`, `file:`).

## Failure & Repair

If validation fails:
1. Keep the same intent and composition.
2. Remove policy-violating tags/attributes.
3. Return one corrected single `<svg>` document only.
