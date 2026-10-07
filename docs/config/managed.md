# Settings tuisana writes

Four sections of `tuisana.toml` are the app's own save file rather than your
settings. tuisana writes them as you work, and reads them back at startup so a
session resumes where the last one ended.

| section | written by | page |
| --- | --- | --- |
| `[[filter_set]]` | `w`, `d`, and every edit while a set is loaded | [Named filter sets](/config/filter-sets) |
| `[view]` | any keystroke that opens or closes a pane | [Saved view state](/config/view) |
| `[[project]]` | `*` and `h` in the project list | [Project visibility](/config/projects) |
| `[gantt]` | committing the Gantt colour dialog | [Gantt colours](/config/gantt) |

## Rules that apply to all four

**They are readable and writable by hand.** The file is read back exactly as
written, and a hand-written entry is treated no differently from one the app
produced.

**But your formatting will not survive.** When tuisana rewrites the file it
re-serializes it, which discards comments, key order, and whitespace. If you
hand-edit one of these sections, expect the next keystroke that changes it to
reformat the file.

**Defaults are omitted.** Anything sitting at its default value is left out, so
a simple entry stays short.

**Writes are batched, not per-keystroke.** A burst of typing produces one
write once it settles, and an edit still in hand is flushed before anything
replaces the panel it was typed into.

## What is not saved

Scroll offsets, the cursor position, the sort, the grouping, the completed
filter, subtask visibility, which pane had focus, the height you resized a pane
to, and the Gantt chart's timeline window.

The line is between *what you set up* and *where you were looking*. A pane
height tuned to one terminal is noise in another.

## Keeping it in version control

`tuisana.toml` holds your personal access token, so it is in this repository's
`.gitignore` — along with the `tuisana.backup*.toml` files the config migration
writes, which are byte-for-byte copies carrying the same token.

If you want to track your settings anyway, note that these four sections churn
on ordinary use. `[gantt.order]` is stored as a sorted map specifically so it
does not produce spurious diffs, but the other three will move.
