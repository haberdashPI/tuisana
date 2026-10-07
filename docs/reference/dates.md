# Date expressions

The same grammar is accepted by the `Due` and `Start` filter rows, by a task's
date cells, and by the calendar overlay's text field.

## Forms

| form | example |
| --- | --- |
| a full date | `2026-09-15` |
| a date in the current year | `09-15` |
| a keyword | `today`, `tomorrow`, `yesterday` |
| a weekday of the current week (Sunday-first) | `mon`, `friday` |
| a whole week, Sunday to Saturday | `this week`, `last week`, `next week` |
| a whole month | `this month`, `last month`, `next month` |
| a whole year | `this year`, `last year`, `next year` |
| an offset, in the name's own unit | `today-5`, `next month+2`, `mon+7` |
| an inclusive range | `2026-09-01..2026-09-30` |
| a range open on one side | `today..`, `..2026-12-31` |

Case and the space are ignored, so `ThisMonth` works.

## Names that cover more than a day

A name covering a span **is a range on its own**: `this month` matches every
day of the month.

Written between two of them, `..` **unions their spans** rather than joining
their first days. Each end contributes its *outer* edge, so
`last month..this month` runs from the first of one to the last of the other,
and `last week..this week` covers both weeks whole.

## Offsets

An offset steps in the unit its name is written in. `today-5` is five days
back; `this month-1` is another spelling of `last month`.

`+` is never a date separator, so it can offset a written date:
`2026-09-15+5`. `-` *is* a separator, so it only offsets a name.

## Names are re-read every time

A **filter** stores the word, not the date it resolved to. A saved filter of
`due: this month` means October in October and November in November.

A **task date** does the opposite. Typing `tomorrow` into a due-date cell sends
the day it resolved to on the spot, and it never moves again — Asana stores a
date, not an expression. A name covering a span commits as the day it *starts*
on there, since a task has one due date and nowhere to put the rest.

## Time zones

Asana's due and start dates are **date-only** values. They carry no time zone
and are never converted.

What *is* resolved in your local zone is today's date — which is what `Today`,
`Tomorrow`, the overdue marker, and the `today` keyword are measured against.

`TUISANA_TODAY=YYYY-MM-DD` pins that date. It is mostly useful for tests and
screenshots.

## What gets pushed to the API

A `due` filter is also sent to Asana as `due_on.after` / `due_on.before`, so
narrowing it reduces what gets downloaded.

With more than one filter set the window sent is the **union** of theirs. A set
that is open-ended, unfiltered, asking for an empty due date, or negating
either the due row or itself opens that side of the window — otherwise the
server would drop rows that set had asked for.

A negation on a `due` row, or on a set containing one, stops that window being
pushed at all: the negated form is satisfied by dates *outside* the window, and
a window narrower than the truth would be cached as covered.

## The calendar overlay

Editing a date filter opens a calendar. It starts on the current month with
today highlighted.

- The grid runs `Su Mo Tu We Th Fr Sa`. The Gantt chart's weekends and week
  boundaries still count from Monday, because a work week and a calendar grid
  are read differently.
- **The two directions stay in step.** Typing `2026-09` moves the grid to
  September even though no day is named yet; a year alone shows that January.
  Moving the highlight rewrites the date you are on — and only that one, so the
  other end of a range is left alone.
- With the grid up, only the characters of a written date type: digits, `-`,
  `+`, and `.`. Every letter is either a key that steers the grid or one stroke
  of a name whose other strokes are.
- **Hiding the grid frees the letters.** With it away, `h`, `l`, `j`, `k`, `t`,
  and `d` go back to being characters, which is what lets a name like `tue` or
  `next month` be typed at all. In the grid's place the overlay shows the dates
  the text resolves to and a key to the names. The choice is remembered for the
  next date you edit.

### How a range is drawn

A range shades the days between its two ends, with both ends picked out. So
does a name covering more than one day: `this month` shades its month. The
shading is the union of the two ends' spans, which is what the filter
matches — `last week..this week` shades all fourteen days.

The highlight sits on the edge the caret's end of the range contributes: its
first day before the `..`, its last day after it. So the caret in `this week`
of `this week..next week` highlights the Sunday the range opens on, and moving
it past the `..` highlights the Saturday it closes on — in both cases the day
the keys are about to move.

A bare name with no `..` is edited as the *end*, and moving it spells the name
out: nudging `this week` by a day gives `2026-08-23..2026-08-30`, keeping the
start it came with. A name covering a single day, like `today`, just becomes
that other day.
