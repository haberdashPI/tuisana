# Project visibility

```toml
[[project]]
gid = "1201"
starred = true
hidden = false

[[project]]
gid = "1202"
starred = false
hidden = true
```

One repeated entry per project, keyed by Asana project GID. Written by the
project list's star and hide keys; rarely worth editing by hand, since you need
the GIDs.

## Keys

| key | default | meaning |
| --- | --- | --- |
| `gid` | — | the Asana project GID |
| `starred` | `false` | sort the project ahead of unstarred ones |
| `hidden` | `false` | keep the project in the hidden group |

## How visibility works

Hidden projects stay available in the list. They are shown after the visible
projects, and only once you toggle the hidden group on — the default key for
which is `v`. When shown, they are marked explicitly so they are easy to spot.

## Why this is not part of a filter set

A [named filter set](/config/filter-sets) carries the project *selection*, but
not these flags. The distinction is deliberate:

> A star is a fact about a project. A selection is a fact about a question.

Starring and hiding keep writing `[[project]]`, which is one record per project
for the whole app, no matter which filter set is loaded. "Selected only" and
the hidden-group toggle stay out of saved sets for the same reason — both are
ways of looking at the project *list*, not parts of the question being asked.
