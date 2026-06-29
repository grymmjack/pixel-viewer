# Viewer status-bar cleanup

Design notes + mockups for the status-bar declutter.

| File | What it shows |
| --- | --- |
| `00-reported-overlap.png` | The reported bug: the viewer's bottom bar overflows and the left/right button groups paint over each other (`rid f 1:1 ⤢ Fit·Tilepack`). |
| `01-diagnosis.png` | Why it happens — a left group that grows right and a right group that grows left, neither truncating, plus the ~18 widgets crammed into one row (three unrelated concerns: NAV / FX / PLAY). |
| `02-proposed-passes.png` | Four cleanup passes (A–D). |

## Passes

- **A — collapse by concern (implemented).** Fold the retro-effect cluster into a
  `📺 CRT` popover and the slideshow/screensaver cluster into a `▶ Auto` popover;
  keep only navigation + zoom inline. Overlap becomes structurally impossible.
- **B — reclaim right-edge width (implemented, light).** Replace the always-on
  `Z+1-0 / Ctrl+wheel…` hint with a compact `?` tooltip (~200px back).
- **C — de-duplicate the playback readout (implemented).** The byte position was
  printed three times; the slider already shows it, so the label is now
  `{pct}% · {size} · {baud} baud`.
- **D — merge the favorites strip into the breadcrumb row (implemented).** The
  near-empty `Favorites: ★ Pin` row is gone; `★ Pin` + the `📁` chips now sit at
  the right end of the path row, reclaiming a full row of vertical height.

Further responsive work (glyph-collapse + `⋯` overflow below ~700px) is a
possible follow-up.
