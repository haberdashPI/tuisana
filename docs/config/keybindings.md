# Key bindings

```toml
[[bind]]
key = "j"
command = "move_down"

[[bind]]
key = "ctrl-j"
mode = "filter_edit"
command = "filter_done_editing"
```

Each `[[bind]]` maps one key to one command, optionally in one mode.

**Your bindings are merged over the defaults, not substituted for them.** If
you omit a command from your config, its built-in binding still applies — so a
config holding three `[[bind]]` entries changes three keys and leaves the rest
of the app alone.

To find a command's name, press `?` in the app: the help overlay for each mode
lists the keys that are actually bound in your config.

## Fields

| field | required | meaning |
| --- | --- | --- |
| `key` | yes | the keystroke, in the spelling below |
| `command` | yes | one of the names in [Commands](/reference/commands) |
| `mode` | no | restrict the binding to one UI context; defaults to `any` |

## Key names

A **single character is itself**: `j`, `?`, `<`, `0`.

**Named keys** are `enter`, `esc`, `backspace`, `space`, `tab`, `home`, `end`,
`left`, `right`, `up`, `down`, `pageup`, and `pagedown`.

**Modifiers are prefixes**: `ctrl-x` for Control, `alt-x` for Option/Alt.

`ctrl-` and `alt-` take exactly **one character** after the prefix, so
`ctrl-enter` and `alt-left` are not bindable. The one modified named key is
`shift-tab` (also spelled `backtab`).

### Case matters — but only on a single letter

`J` and `j` are different keys. That is what edit mode's `I`, `J`, `K`, `S`,
and `X` are bound on.

Case does **not** matter anywhere else. `ESC` is `esc`, and `ctrl-J` is
`ctrl-j`, because a terminal cannot reliably tell it from the lowercase form —
binding the uppercase version would be binding a key that never fires. (It is
also why the subtask key is `I` rather than `ctrl-i`, which is the byte `tab`.)

Punctuation that is itself shift-produced — `?`, `!`, `@`, `*`, `<`, `>`, `{`,
`}` — arrives as its own character and is unaffected.

### `alt-` needs terminal support

`alt-` requires your terminal to send Option as a **modifier** rather than as
an escape prefix:

- **macOS Terminal**: Profiles → Keyboard → "Use Option as Meta key"
- **iTerm2**: Profiles → Keys → Left/Right Option key → `Esc+`

The escape-prefix form is not supported: a lone `ESC` is `esc` here, and
telling the two apart by timing is how editors get famously confused. Every
default binding that uses `alt-` is rebindable if your terminal cannot send it.

## Modes

| mode | when it is active |
| --- | --- |
| `any` | the fallback, used when no more specific binding matches |
| `project` | the project list |
| `project_search` | typing a project search string |
| `filter` | the filter panel, browsing rows |
| `filter_edit` | a filter row is open for editing |
| `filter_set_name` | naming a set for `w`, or answering the `d` confirmation |
| `calendar` | the date picker |
| `task` | the task table |
| `edit` | the structural edits — insert, delete, move between sections |
| `column_edit` | a table cell is open for editing |
| `confirm` | a bulk edit is waiting on `y` or `n` |
| `gantt` | the timeline |
| `gantt_order` | the Gantt colour dialog |

### Which modes fall back to `any`

Most modes do. These do **not**: `project_search`, `filter_edit`,
`filter_set_name`, `calendar`, `column_edit`, and `confirm`.

All six either accept typed text or own the screen. An unbound letter in a cell
editor has to type into the cell rather than fire whatever global command that
letter carries, and a confirmation that acted on `j` would be worse than no
confirmation at all.

::: tip Legacy spelling
`mode = "task_edit"` is accepted as an alias for `column_edit`, so a
version-1 file parses at all — see [Config format versions](/reference/migrations).
:::

## Default bindings

The complete built-in set. Anything you bind replaces the entry for that key
and mode; everything else below stays in force.

### Global (`mode` omitted)

Active in every mode that does not override the key. Modes that swallow every key — project search, filter edit, the calendar, a cell editor, and a confirmation — do not fall back here, because an unbound letter there has to type rather than fire a global command.

| key | command |
| --- | --- |
| `?` | `toggle_help_details` |
| `q` | `quit` |
| `ctrl-c` | `quit` |
| `k` | `move_up` |
| `up` | `move_up` |
| `j` | `move_down` |
| `down` | `move_down` |
| `ctrl-u` | `page_up` |
| `ctrl-d` | `page_down` |
| `home` | `jump_top` |
| `end` | `jump_bottom` |
| `left` | `scroll_left` |
| `right` | `scroll_right` |
| `f` | `set_filter_mode` |
| `p` | `set_project_mode` |
| `t` | `set_task_mode` |
| `m` | `toggle_task_mode` |
| `[` | `resize_top_pane_down` |
| `]` | `resize_top_pane_up` |
| `{` | `minimize_top_pane` |
| `}` | `maximize_top_pane` |
| `0` | `restore_top_pane` |
| `r` | `refresh` |

### Project mode (`project`)

The project list.

| key | command |
| --- | --- |
| `enter` | `open` |
| `/` | `start_search` |
| `space` | `toggle_selection` |
| `a` | `select_all_visible` |
| `!` | `select_all_starred_visible` |
| `@` | `select_all_non_hidden_visible` |
| `i` | `invert_selection` |
| `c` | `clear_selection` |
| `u` | `undo_selection` |
| `ctrl-y` | `redo_selection` |
| `*` | `toggle_starred_selected` |
| `h` | `toggle_hidden_selected` |
| `v` | `toggle_hidden_group` |
| `b` | `filter_sets_toggle` |
| `w` | `filter_set_save` |
| `y` | `filter_set_copy_to_new` |
| `n` | `filter_set_new` |
| `d` | `filter_set_delete` |
| `<` | `filter_sets_page_back` |
| `>` | `filter_sets_page_forward` |
| `1` | `filter_set_load_1` |
| `2` | `filter_set_load_2` |
| `3` | `filter_set_load_3` |
| `4` | `filter_set_load_4` |
| `5` | `filter_set_load_5` |
| `6` | `filter_set_load_6` |
| `7` | `filter_set_load_7` |
| `8` | `filter_set_load_8` |
| `9` | `filter_set_load_9` |
| `o` | `toggle_only_selected` |
| `ctrl-z` | `search_fuzzy` |
| `ctrl-s` | `search_substring` |
| `ctrl-r` | `search_regex` |

### Project search mode (`project_search`)

Typing a search string. Ordinary characters, `backspace`, `enter`, and `esc` are handled directly.

| key | command |
| --- | --- |
| `ctrl-l` | `clear_search` |

### Filter mode (`filter`)

The filter panel, browsing rows rather than editing one.

| key | command |
| --- | --- |
| `enter` | `begin_filter_edit` |
| `esc` | `set_task_mode` |
| `f` | `toggle_task_filters` |
| `s` | `cycle_filter_string_mode` |
| `ctrl-l` | `clear_search` |
| `ctrl-z` | `search_fuzzy` |
| `ctrl-s` | `search_substring` |
| `ctrl-r` | `search_regex` |
| `l` | `filter_set_next` |
| `h` | `filter_set_prev` |
| `a` | `filter_set_add` |
| `x` | `filter_set_remove` |
| `e` | `filter_require_empty` |
| `!` | `filter_negate_field` |
| `~` | `filter_negate_set` |
| `b` | `filter_sets_toggle` |
| `w` | `filter_set_save` |
| `y` | `filter_set_copy_to_new` |
| `n` | `filter_set_new` |
| `d` | `filter_set_delete` |
| `<` | `filter_sets_page_back` |
| `>` | `filter_sets_page_forward` |
| `1` | `filter_set_load_1` |
| `2` | `filter_set_load_2` |
| `3` | `filter_set_load_3` |
| `4` | `filter_set_load_4` |
| `5` | `filter_set_load_5` |
| `6` | `filter_set_load_6` |
| `7` | `filter_set_load_7` |
| `8` | `filter_set_load_8` |
| `9` | `filter_set_load_9` |

### Filter edit mode (`filter_edit`)

A filter row is open for editing.

| key | command |
| --- | --- |
| `ctrl-q` | `filter_require_empty` |
| `ctrl-n` | `filter_negate_field` |
| `ctrl-t` | `filter_negate_set` |
| `enter` | `filter_done_editing` |
| `esc` | `filter_cancel_editing` |
| `ctrl-l` | `clear_search` |
| `ctrl-z` | `search_fuzzy` |
| `ctrl-s` | `search_substring` |
| `ctrl-r` | `search_regex` |
| `h` | `filter_move_label_left` |
| `l` | `filter_move_label_right` |
| `j` | `filter_cycle_label_down` |
| `k` | `filter_cycle_label_up` |
| `a` | `filter_add_label` |
| `d` | `filter_delete_label` |
| `left` | `filter_caret_left` |
| `right` | `filter_caret_right` |
| `ctrl-b` | `filter_caret_left` |
| `ctrl-f` | `filter_caret_right` |
| `tab` | `complete_next_candidate` |
| `shift-tab` | `complete_prev_candidate` |
| `alt-b` | `text_caret_word_back` |
| `alt-f` | `text_caret_word_forward` |
| `ctrl-a` | `text_caret_start` |
| `ctrl-e` | `text_caret_end` |
| `ctrl-d` | `text_cut_char` |
| `alt-d` | `text_cut_word` |
| `ctrl-k` | `text_cut_to_end` |
| `ctrl-p` | `highlight_prev_candidate` |

### Calendar mode (`calendar`)

The date picker, over a filter row or a task's date cell.

| key | command |
| --- | --- |
| `h` | `calendar_prev_day` |
| `l` | `calendar_next_day` |
| `k` | `calendar_prev_month` |
| `j` | `calendar_next_month` |
| `t` | `calendar_today` |
| `d` | `calendar_clear` |
| `;` | `calendar_toggle_grid` |
| `enter` | `calendar_commit` |
| `esc` | `calendar_close` |
| `left` | `filter_caret_left` |
| `right` | `filter_caret_right` |
| `ctrl-b` | `filter_caret_left` |
| `ctrl-f` | `filter_caret_right` |
| `ctrl-a` | `calendar_jump_to_start` |
| `ctrl-e` | `calendar_jump_to_end` |
| `?` | `toggle_help_details` |

### Task mode (`task`)

The task table.

| key | command |
| --- | --- |
| `[` | `move_section_up` |
| `]` | `move_section_down` |
| `{` | `move_project_up` |
| `}` | `move_project_down` |
| `c` | `toggle_completed_filter` |
| `z` | `toggle_subtask_visibility` |
| `,` | `toggle_project_grouping` |
| `.` | `toggle_section_grouping` |
| `s` | `toggle_column_sort` |
| `o` | `open` |
| `space` | `toggle_task_selection` |
| `a` | `select_all_visible_tasks` |
| `i` | `invert_task_selection` |
| `x` | `clear_task_selection` |
| `ctrl-x` | `clear_hidden_task_selection` |
| `y` | `copy_tasks_to_clipboard` |
| `g` | `set_gantt_mode` |
| `b` | `toggle_recent_pane` |
| `h` | `task_column_prev` |
| `l` | `task_column_next` |
| `enter` | `edit_column` |
| `d` | `toggle_task_completed` |
| `t` | `set_edit_mode` |

### Edit mode (`edit`)

The structural edits: the keys that change which rows exist.

| key | command |
| --- | --- |
| `i` | `insert_task` |
| `I` | `insert_subtask` |
| `x` | `mark_for_deletion` |
| `X` | `delete_section` |
| `S` | `insert_section` |
| `J` | `move_task_to_next_section` |
| `K` | `move_task_to_prev_section` |
| `space` | `toggle_task_selection` |
| `enter` | `delete_marked_tasks` |
| `esc` | `edit_cancel` |

### Column edit mode (`column_edit`)

A task table cell is open for editing.

| key | command |
| --- | --- |
| `enter` | `commit_column_edit` |
| `esc` | `cancel_column_edit` |
| `j` | `column_edit_next_value` |
| `k` | `column_edit_prev_value` |
| `d` | `column_edit_clear` |
| `ctrl-l` | `column_edit_clear` |
| `left` | `filter_caret_left` |
| `right` | `filter_caret_right` |
| `ctrl-b` | `filter_caret_left` |
| `ctrl-f` | `filter_caret_right` |
| `alt-b` | `text_caret_word_back` |
| `alt-f` | `text_caret_word_forward` |
| `ctrl-a` | `text_caret_start` |
| `ctrl-e` | `text_caret_end` |
| `ctrl-d` | `text_cut_char` |
| `alt-d` | `text_cut_word` |
| `ctrl-k` | `text_cut_to_end` |
| `tab` | `complete_next_candidate` |
| `shift-tab` | `complete_prev_candidate` |
| `ctrl-n` | `highlight_next_candidate` |
| `ctrl-p` | `highlight_prev_candidate` |

### Gantt mode (`gantt`)

The timeline beside the table.

| key | command |
| --- | --- |
| `esc` | `set_task_mode` |
| `g` | `toggle_gantt` |
| `<` | `gantt_remove_column` |
| `>` | `gantt_add_column` |
| `c` | `cycle_gantt_color_key` |
| `h` | `gantt_scroll_left` |
| `l` | `gantt_scroll_right` |
| `-` | `gantt_zoom_out` |
| `=` | `gantt_zoom_in` |
| `+` | `gantt_zoom_in` |
| `z` | `gantt_zoom_fit` |
| `t` | `gantt_today` |
| `enter` | `gantt_open_order` |

### Gantt order mode (`gantt_order`)

The colour dialog.

| key | command |
| --- | --- |
| `j` | `move_down` |
| `k` | `move_up` |
| `ctrl-j` | `gantt_order_move_down` |
| `ctrl-k` | `gantt_order_move_up` |
| `t` | `gantt_order_move_top` |
| `b` | `gantt_order_move_bottom` |
| `c` | `cycle_gantt_color_key` |
| `enter` | `gantt_order_commit` |
| `esc` | `gantt_order_cancel` |

## Two bindings worth explaining

**`ctrl-n` in filter edit mode** is bound to `filter_negate_field` and keeps
that meaning — except with a completion overlay open, where `ctrl-n` is the
obvious key for "next candidate" and walks the candidates instead. `!` on the
row in filter-browse mode is the other way to negate it.

**`search_fuzzy` is on `ctrl-z`, not `ctrl-f`**, so that `ctrl-b` and `ctrl-f`
can move the caret in every mode that edits text.
