# Config format versions

```toml
[header]
type = "tuisana"
version = 3.0
```

The header identifies the config format. Both values are validated at startup;
leave them alone.

Version 3 is current. **Versions 1 and 2 are still readable and are migrated
once.** A file with no `[header]` at all is read as version 1, not as the
current version. A file claiming a version from the future is rejected.

## The migration prompt

There are two changes, and a file gets whichever ones it is behind on. Rather
than let either happen quietly, the first run against an older config asks
before touching it:

```
┌ Config format changed ──────────────────────────┐
│                                                 │
│ tuisana.toml is in the version 1 format.        │
│ Six command names and one mode name have        │
│ been renamed, and uppercase letters are now     │
│ distinct keys.                                  │
│                                                 │
│ The selected projects move from [view] onto     │
│ the filter sets, which is where a selection     │
│ now lives.                                      │
│                                                 │
│ tuisana will rewrite it in the version 3 format.│
│ A backup would go to tuisana.backup.toml.       │
│                                                 │
│ y        back it up first, then migrate         │
│ n        migrate without a backup               │
│ q / esc  quit and change nothing                │
│                                                 │
└─────────────────────────────────────────────────┘
```

A version-2 file sees only the second paragraph: the window describes the
changes that are actually going to happen to the file in front of it.

- **`y`** copies the file byte for byte first, so the backup keeps your
  comments, key order, and formatting — none of which survive being
  re-serialized. An existing `tuisana.backup.toml` is not clobbered; the next
  free name is `tuisana.backup.2.toml`, and the window names the file it will
  actually write.
- **`n`** migrates in place with no backup.
- **`q` / `esc`** quit without touching anything, and the prompt returns next
  time. Nothing can write the config while it is up, so a file you meant to
  edit by hand is still the file you left.

::: warning The backup carries your token
`tuisana.backup.toml` is a byte-for-byte copy, so it holds the same personal
access token. The repository's `.gitignore` already covers
`tuisana.backup*.toml`.
:::

## Version 2

Renamed six commands and one mode, and made shift+letter a key namespace of its
own — so a version-1 binding of `key = "G"`, which used to mean `g`, would
silently become a different key.

| version 1 | version 2 |
| --- | --- |
| `mode = "task_edit"` | `mode = "column_edit"` |
| `begin_task_edit` | `edit_column` |
| `commit_task_edit` | `commit_column_edit` |
| `cancel_task_edit` | `cancel_column_edit` |
| `task_edit_next_value` | `column_edit_next_value` |
| `task_edit_prev_value` | `column_edit_prev_value` |
| `task_edit_clear` | `column_edit_clear` |

All seven old spellings are still *parsed* as aliases — the migration is what
rewrites them, and it cannot run on a file that failed to deserialize in the
first place.

## Version 3

Moved the project selection out of `[view]` and onto the
[filter sets](/config/filter-sets), which is the only place one lives now.

A bound entry adopts the list that was sitting beside it. With nothing bound,
the list goes to the reserved `scratch` entry — so the projects you were
working with come back either way.

## Other accepted aliases

Independent of the version migration, a handful of older command spellings are
still accepted:

| alias | current name |
| --- | --- |
| `set_project_bind_mode` | `set_project_mode` |
| `set_task_bind_mode` | `set_task_mode` |
| `minimize_window` | `minimize_top_pane` |
| `maximize_window` | `maximize_top_pane` |
| `restore_window` | `restore_top_pane` |
| `resize_window_up` | `resize_top_pane_up` |
| `resize_window_down` | `resize_top_pane_down` |
