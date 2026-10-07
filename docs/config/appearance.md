# Appearance

```toml
[theme]
variant = "ansi"     # "ansi" | "truecolor" | "mono"
glyphs = "unicode"   # "unicode" | "ascii"
accent = "cyan"
zebra = false
```

Every value has a sensible default, so `[theme]` can be left out entirely. The
block above *is* the defaults.

## `variant`

How much colour tuisana uses.

| value | what it does |
| --- | --- |
| `"ansi"` | the terminal's own 16-colour palette, so the UI inherits whatever scheme you already use. **Default.** |
| `"truecolor"` | adds indexed shades for subtle backgrounds. Required by `zebra`. |
| `"mono"` | no colour at all; relies on bold, dim, and reverse video. |

Setting the `NO_COLOR` environment variable forces `"mono"`, together with the
ASCII glyph set.

Everything tuisana marks is marked without relying on colour alone. A negated
filter row swaps its gutter marker for `¬`; a row marked for deletion is drawn
struck through; in `"mono"` the Gantt chart tells its six palette slots apart
by bar texture rather than hue. The chart stays readable with no colour
whatsoever.

## `glyphs`

| value | what it does |
| --- | --- |
| `"unicode"` | box-drawing characters and Unicode markers. **Default.** |
| `"ascii"` | an ASCII equivalent for every box-drawing and marker glyph. |

Use `"ascii"` for terminals whose fonts render ambiguous-width characters as
two cells, which otherwise shifts columns out of alignment.

## `accent`

The colour used for the focused pane's border, the mode badge, and dates due
within three days.

One of: `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`,
`gray`. Defaults to `cyan`.

## `zebra`

`true` stripes alternating task rows with a subtle background. Requires
`variant = "truecolor"`; it has no effect otherwise. Defaults to `false`.

## Reading the screen

A short guide to what the chrome means, since it is not all self-evident:

- The **focused pane** has a thick border, tinted with the current mode's
  colour. The mode badge at the bottom left uses the same colour.
- The **header bar** names the app and the current project or task context.
- The **hint bar**, above the mode badge, lists the most useful keys for the
  current mode — resolved from your actual bindings, so it stays correct after
  you rebind things.
- The **status bar** lists only the settings that differ from their defaults,
  so an untouched session shows nothing there.
- Pane titles and counts live in the pane's own border.
- A **spinner** at the top left reports work in progress: `loading` while tasks
  are being fetched, `filtering` when the table has fallen behind what you have
  typed. The second only appears once the wait is long enough to notice.
- In the task table the left gutter carries two markers: the cursor (`▍`) and
  multi-selection (`●`). An empty cell shows `—`.
- **Due dates render relatively** — `Today`, `Tomorrow`, `Wed`, `Nov 14`. A
  weekday name means a day of the week you are in now, which turns over on
  Sunday, so it can point backwards; anything outside that week gets its date.
  An overdue date is prefixed with `!` and coloured red, one due today is
  yellow, and one within three days takes the accent colour.

## Reading the Gantt chart

The chart's glyphs are the one part of the UI the in-app help does not explain,
since `?` lists keys rather than symbols:

| glyph | meaning |
| --- | --- |
| `█` | a bar, from the start date to the due date |
| `▒` | a bar whose value did not get one of the six colours |
| `◆` | a milestone: a due date with no start date |
| blank | the task has no dates at all |
| `▼` `┊` | today, on the axis and drawn through rows with no bar there |
| `·` | an axis mark, on group heading rows |
| `░` | a Saturday or Sunday, wherever no bar covers it |
| `‹` `›` | the bar runs past the window, or the task is entirely outside it |

The axis marks whatever it has room for. A year or a quarter is marked by
month; zoom in until a month fits the pane and it marks weeks; again and it
marks individual days; again and each day gets its weekday name, so the axis
reads `Sat 22  Sun 23  Mon 24`. Every step adds to the one below it — zooming
in never trades the date away for the weekday.

Once there is at least one column per day, Saturdays and Sundays are shaded. A
bar drawn across a weekend covers the shading, so what shows through is exactly
the weekends a piece of work did not run through.

With `variant = "mono"` the six palette slots are told apart by the bar's
texture rather than its colour, so the chart still reads with no colour at all.

See [Gantt colours](/config/gantt) for which dimension the bars take their
colour from.
