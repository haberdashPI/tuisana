# Editing behaviour

```toml
[edit]
confirm_threshold = 5
```

One knob, because there is only one judgement call to make: how many rows count
as "a few".

## `confirm_threshold`

An edit in tuisana applies to **every selected task**, or to the cursor row
when nothing is selected. `confirm_threshold` is how many rows may change
before the app stops and asks.

- **At or below the threshold**, the edit goes on the keystroke.
- **Above it**, a window says how many rows are about to change and waits for
  `y` or `n`. Nothing happens at all — locally or in Asana — until it is
  answered.

Defaults to `5`.

| value | effect |
| --- | --- |
| `0` | confirm every bulk edit, including single-row ones |
| `5` | let five rows through, ask about six (the default) |
| a number larger than any selection you make | turn the confirmation off |

The comparison is strictly greater, so a threshold of `5` lets exactly five
rows through.

## What you see regardless

Independent of the threshold, the column cell of **every row an edit would
change** is banded while the selection stands, and the pane border reads
`editing 3`. The blast radius is on screen before the key is pressed.

The targets are fixed when the editor *opens*, so a table that rebuilds
underneath you cannot widen what `enter` is about to change.

## Pacing, which is not configurable

How many requests go out at once, and how fast, is a property of Asana's rate
limits rather than a matter of taste. It lives in the source as constants and
has no config key.
