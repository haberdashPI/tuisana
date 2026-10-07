# The config file

tuisana reads a single TOML file, **`tuisana.toml`, from the current working
directory**. It is both your settings file and the app's own save file, which
is the first thing to understand about it.

## Two halves

The sections divide cleanly into ones you write and ones tuisana writes.

### Settings you edit

tuisana never touches these. Write them by hand; they are read once at startup.

| section | what it holds |
| --- | --- |
| [`[auth]`](/config/auth) | your Asana personal access token, and optionally a workspace |
| [`[theme]`](/config/appearance) | colours and glyphs |
| [`[[bind]]`](/config/keybindings) | key → command mappings |
| [`[edit]`](/config/editing) | how wide a bulk edit may be before it asks |

### Settings tuisana writes

These are the app's memory of where you left off. You *can* edit them by
hand — they are read back exactly as written — but the next keystroke that
changes the thing they describe will rewrite them, discarding your comments and
key order along the way.

| section | written by |
| --- | --- |
| [`[[filter_set]]`](/config/filter-sets) | `w`, `d`, and every edit while a set is loaded |
| [`[view]`](/config/view) | any keystroke that opens or closes a pane |
| [`[[project]]`](/config/projects) | `*` and `h` in the project list |
| [`[gantt]`](/config/gantt) | committing the Gantt colour dialog |

See [Settings tuisana writes](/config/managed) for the rules that apply to all
four.

## The header

Every file carries a header identifying the format:

```toml
[header]
type = "tuisana"
version = 3.0
```

Leave both values alone. They are validated at startup, and `version` is what
lets tuisana recognise and migrate an older file. Versions 1 and 2 are still
readable; see [Config format versions](/reference/migrations) for what changed
and what the migration prompt asks.

## What is deliberately not saved

Scroll offsets, the cursor position, the sort, the grouping, the completed
filter, subtask visibility, which pane had focus, the height you dragged a pane
to, and the Gantt chart's timeline window are all left out.

The line is between *what you set up* and *where you were looking*. A pane
height tuned to one terminal is noise in another; a sort order is how you are
reading an answer, not what you asked.

## A complete example

The repository ships
[`tuisana.toml.example`](https://github.com/haberdashPI/tuisana/blob/main/tuisana.toml.example),
a fully commented file containing every section and every default binding.
Copy it to `tuisana.toml` and fill in your token if you would rather start from
something complete than something minimal.
