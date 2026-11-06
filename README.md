# Seed Map (Horizon native mod)

A Chunkbase-style seed map inside the game, plus a minimap, built on the engine's own Minecraft 26.3
world generation (`horizon-world`). Biomes and structures come from the game's generator for the seed
you type, not from a separate port.

## Use

- **M** (rebindable in Options > Mods) or `/seedmap [seed]` opens the map.
- Type or paste (Ctrl+V) a seed, Enter applies it. A number is used as-is; other text is hashed like
  vanilla's seed box. Overworld / Nether / End buttons. **Me** centres on you; your position and
  facing are drawn.
- Drag to pan, scroll to zoom, WASD/arrows pan, +/- zoom, C centres, Tab focuses the seed field.
- Icons are the game's own item icons (bell, crossbow, sculk shrieker, elytra...). End cities are split
  into **End ship (elytra)** and **End city (no ship)**. Hover for name and coordinates, click an icon or
  right-click the map to copy `X Z`. The legend toggles each type and is remembered.
- A scale bar on the map shows the distance.
- **Minimap** (top left of the HUD): **N** shows or hides it, **]** / **[** zoom, **\\** changes its size.
  The label under it says how many blocks it spans. All keys are rebindable in Options > Mods.

Settings are saved in `<game folder>/config/seedmap.json`.

## Install

Needs a Horizon install on revision `66f3981f522b24c250b21523a70080965b940424` (Minecraft 26.3 nightly
2026-11-05). Download `seedmap-0.2.0.hnmod` from the release, copy it to your instance's
`horizon-native-mods/`, add the line `mod.seedmap:true` to the instance's `horizon-mods.txt`, then build
and restart:

    horizon-launcher native-build --instance <id> --profile performance --trust-native-code

Native mods are compiled into the game and run with your rights: read the source first.

## Limits

- Singleplayer worlds do not expose their seed to the client. Run `/seed`, click the seed in chat to copy
  it, paste it into the field.
- Structures load up to a zoom of 16 blocks per pixel (the work grows with the area); further out you see
  biomes, strongholds and spawn only.
- The map shows the surface: cave biomes are resampled from above ground.
- Not included: slime chunks, a Seed Finder, drawing tools, Bedrock.
- Only built against the revision above; another Horizon revision needs the bundle re-pinned and checked.

## Test status

Tested on one revision with seed 12345 in a singleplayer test world: key and command opening, seed entry and
paste, zoom, Nether/End switching, legend toggles, tooltips, saved settings, minimap, End ship/city icons.
Structure positions were cross-checked with the engine's own `/locate` for a few structures, not against
Chunkbase. The minimap zoom/size keys and drag-panning were not exercised in the live client. Model logic
has unit tests (`devtests/`).

## Development

`stage.sh` applies the bundle to Horizon's source for a compile check, `tools/package.py` writes
`dist/seedmap-<version>.hnmod`, `devtests/` holds the model tests (`cargo test`).

MIT licensed.
