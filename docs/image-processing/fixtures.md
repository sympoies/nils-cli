# image-processing fixtures

## Backend detection
- Missing backend:
  - Setup: PATH without `magick`, `convert`, and `identify`
  - Command: `image-processing info --in a.png --json`
  - Expect: exit `1`, stderr contains `missing ImageMagick (need \`magick\` or both \`convert\` + \`identify\`)`

## Output mode gating
- Missing output mode (output-producing subcommand):
  - Command: `image-processing convert --in a.png --to webp --json`
  - Expect: exit `2`, error mentions `must specify exactly one output mode`
- Multiple output modes:
  - Command: `image-processing resize --in a.png --scale 2 --out x.png --out-dir out --json`
  - Expect: exit `2`
- In-place requires confirmation:
  - Command: `image-processing rotate --in a.png --degrees 90 --in-place --json`
  - Expect: exit `2`, error mentions `--in-place is destructive and requires --yes`

## info
- Command: `image-processing info --in dir --recursive --glob '*.png' --json`
- Expect: JSON `items` length matches resolved inputs; each item `output_path` is `null`.

## auto-orient
- Command: `image-processing auto-orient --in a.jpg --out out/a.jpg --json`
- Expect: output exists; JSON item `status=ok`.

## convert
- Basic:
  - Command: `image-processing convert --in a.png --to webp --out out/a.webp --json`
  - Expect: output ext matches `--to`; JSON item ok.
- Alpha → JPEG requires background (usage error):
  - Command: `image-processing convert --in alpha.png --to jpg --out out/a.jpg --json`
  - Expect: exit `2`, error mentions `alpha input cannot be converted to JPEG without a background`

## resize
- Scale:
  - Command: `image-processing resize --in a.png --scale 2 --out out/a.png --json`
  - Expect: command list includes `-resize 200%` (unless `--no-pre-upscale`).
- Box fit contain requires background for JPEG (runtime item error):
  - Command: `image-processing resize --in a.jpg --width 10 --height 10 --fit contain --out out/a.jpg --json`
  - Expect: exit `1`, item `status=error`, `error` mentions `contain fit requires padding background`

## rotate
- Missing degrees (usage):
  - Command: `image-processing rotate --in a.png --out out/a.png --json`
  - Expect: exit `2`, error mentions `rotate requires --degrees`
- Non-right-angle JPEG requires background (runtime item error):
  - Command: `image-processing rotate --in a.jpg --degrees 13 --out out/a.jpg --json`
  - Expect: exit `1`, item error mentions background requirement.

## crop
- Exactly one mode required (usage):
  - Command: `image-processing crop --in a.png --out out/a.png --json`
  - Expect: exit `2`
- Aspect crop:
  - Command: `image-processing crop --in a.png --aspect 1:1 --out out/a.png --json`
  - Expect: ok.

## pad
- Requires width+height (usage):
  - Command: `image-processing pad --in a.png --out out/a.png --json`
  - Expect: exit `2`

## flip / flop
- Command: `image-processing flip --in a.png --out out/a.png --json`
- Expect: ok.

## optimize
- JPG path:
  - Command: `image-processing optimize --in a.jpg --out out/a.jpg --json`
  - Expect: ok; when `cjpeg`/`djpeg` are present on PATH, commands include a `djpeg | cjpeg` pipeline.
- WEBP path:
  - Command: `image-processing optimize --in a.webp --out out/a.webp --json`
  - Expect: ok; when `cwebp`/`dwebp` are present on PATH, commands include both decode + encode steps.

