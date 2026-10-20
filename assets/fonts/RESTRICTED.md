# Optional faces that are not in this repository

These faces are used by some typography options, but their licences do not allow them to be
redistributed in a source repository, so they are not included here:

| family | files | source | licence |
|---|---|---|---|
| Satoshi | `Satoshi-{Light,Regular,Medium,Bold}.otf` | [Fontshare](https://www.fontshare.com/fonts/satoshi) | Fontshare Free Font License (FFL) |
| Cabinet Grotesk | `CabinetGrotesk-{Light,Regular,Medium,Bold,Extrabold}.otf` | [Fontshare](https://www.fontshare.com/fonts/cabinet-grotesk) | FFL |
| Clash Display | `ClashDisplay-{Medium,Semibold,Bold}.otf` | [Fontshare](https://www.fontshare.com/fonts/clash-display) | FFL |
| Panchang | `Panchang-{Bold,Extrabold}.otf` | [Fontshare](https://www.fontshare.com/fonts/panchang) | FFL |
| Zodiak | `Zodiak-{Thin,Light,LightItalic,Regular,Italic,Bold}.otf` | [Fontshare](https://www.fontshare.com/fonts/zodiak) | FFL |
| Frantically | `Frantically-Regular.ttf` | [1001fonts](https://www.1001fonts.com/frantically-font.html) | 1001fonts freeware EULA |

## Without them

Nothing breaks. When one of these files is missing, every font loader reads an open, bundled
fallback from this folder instead (`restricted_fallbacks` in `registry.json`):

| restricted face | open fallback |
|---|---|
| Satoshi Light / Regular / Medium | Lato Regular |
| Satoshi Bold | Lato Bold |
| Cabinet Grotesk Light / Regular | Fira Sans Regular |
| Cabinet Grotesk Medium | Barlow Medium |
| Cabinet Grotesk Bold | Poppins Bold |
| Cabinet Grotesk Extrabold | Archivo Black |
| Clash Display Medium | Poppins Medium |
| Clash Display Semibold | Poppins Bold |
| Clash Display Bold | Archivo Black |
| Panchang Bold | Poppins ExtraBold |
| Panchang Extrabold | Poppins Black |
| Zodiak Thin / Light | Instrument Serif Regular |
| Zodiak Light Italic | Instrument Serif Italic |
| Zodiak Regular / Bold | DM Serif Display Regular |
| Zodiak Italic | DM Serif Display Italic |
| Frantically | Zilla Slab SemiBold Italic |

Typography choices do not change: an option built on a restricted face stays available, and the
compiled scene still names the original face. Only the glyphs drawn differ. `motion-engine fonts
list` marks such faces `fallback` and names the file it reads.

## Getting them

```
scripts/fetch_fonts.sh            # Fontshare families into assets/fonts (accepts the FFL)
```

Frantically has no stable download URL: download it from 1001fonts and copy
`Frantically-Regular.ttf` into `assets/fonts/`. Fetched files are used as soon as they are present.
