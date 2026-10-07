# Commands

Every name that can appear as `command` in a [`[[bind]]`](/config/keybindings)
entry, grouped by the mode it belongs to. The `mode` column of the
[default bindings table](/config/keybindings#default-bindings) shows where each
one is bound out of the box.

A command bound in a mode where it means nothing is simply inert.

## Global

Bindable with no `mode`, and available in every mode that falls back to `any`.

| command | what it does |
| --- | --- |
| `quit` | leave the app |
| `refresh` | re-fetch the selected projects' tasks |
| `toggle_help_details` | open or close the help overlay for the current mode |
| `move_up` / `move_down` | move the cursor one row |
| `page_up` / `page_down` | move the cursor one screen |
| `jump_top` / `jump_bottom` | move the cursor to the first or last row |
| `scroll_left` / `scroll_right` | scroll the table's columns |
| `set_project_mode` | show the project list in the top pane |
| `set_filter_mode` | show the filter panel in the top pane |
| `set_task_mode` | put the cursor in the task table |
| `toggle_task_mode` | show or hide the task table |
| `resize_top_pane_up` / `resize_top_pane_down` | grow or shrink the top pane |
| `minimize_top_pane` | collapse the top pane and move to the task table |
| `maximize_top_pane` | give the top pane the whole screen |
| `restore_top_pane` | return the top pane to its normal height |

## Project mode

| command | what it does |
| --- | --- |
| `open` | open the project under the cursor in Asana |
| `start_search` | begin typing a search string |
| `toggle_selection` | select or deselect the project under the cursor |
| `select_all_visible` | select every project currently listed |
| `select_all_starred_visible` | select the starred ones among them |
| `select_all_non_hidden_visible` | select the non-hidden ones among them |
| `invert_selection` | select what was not selected, and vice versa |
| `clear_selection` | select nothing |
| `undo_selection` / `redo_selection` | step through this session's selection history |
| `toggle_starred_selected` | star or unstar the selection — writes `[[project]]` |
| `toggle_hidden_selected` | hide or unhide the selection — writes `[[project]]` |
| `toggle_hidden_group` | show or hide the hidden-projects group |
| `toggle_only_selected` | narrow the list to the selected projects |

## Project search mode

| command | what it does |
| --- | --- |
| `clear_search` | empty the search string |

Ordinary typing, `backspace`, `enter`, and `esc` are handled directly and are
not bindable.

## Search modes

Bound in project, filter, and filter-edit modes.

| command | what it does |
| --- | --- |
| `search_fuzzy` | match as a subsequence, ranked |
| `search_substring` | match a literal substring |
| `search_regex` | match a regular expression |
| `clear_search` | empty the field |
| `cycle_filter_string_mode` | step a filter row through its available match modes |

## Filter mode

| command | what it does |
| --- | --- |
| `begin_filter_edit` | open the filter row under the cursor for editing |
| `toggle_task_filters` | close the filter panel and return to the task table |
| `filter_require_empty` | require the row's field to have no value |
| `filter_negate_field` | invert the row's verdict |
| `filter_negate_set` | invert the whole set, after its rows have ANDed |
| `filter_set_add` / `filter_set_remove` | add or remove a filter set tab |
| `filter_set_next` / `filter_set_prev` | move between tabs |

## Named filter sets

Bound in **both** project and filter modes — the sidebar and its keys work in
either.

| command | what it does |
| --- | --- |
| `filter_sets_toggle` | show or hide the `Sets` sidebar |
| `filter_set_load_1` … `filter_set_load_9` | load the entry at that position in the sidebar |
| `filter_sets_page_back` / `filter_sets_page_forward` | page the sidebar when entries overflow |
| `filter_set_save` | save the panel under a name, and bind to it |
| `filter_set_copy_to_new` | copy the panel to a new unnamed one, leaving the entry as it was |
| `filter_set_new` | discard the panel and start from nothing |
| `filter_set_delete` | delete the loaded entry, after confirmation |

## Filter edit mode

| command | what it does |
| --- | --- |
| `filter_done_editing` | commit the edit and return to filter mode |
| `filter_cancel_editing` | discard the edit, close the panel, return to task mode |
| `filter_caret_left` / `filter_caret_right` | move the caret one character |
| `text_caret_word_back` / `text_caret_word_forward` | move the caret one word |
| `text_caret_start` / `text_caret_end` | move the caret to the ends of the line |
| `text_cut_char` | cut the character at the caret |
| `text_cut_word` | cut the word ahead of the caret |
| `text_cut_to_end` | cut to the end of the line |
| `filter_move_label_left` / `filter_move_label_right` | move between a label field's slots |
| `filter_cycle_label_up` / `filter_cycle_label_down` | cycle the slot through its options |
| `filter_add_label` / `filter_delete_label` | add or remove a label slot |
| `complete_next_candidate` / `complete_prev_candidate` | complete the typed prefix to the next candidate |
| `highlight_next_candidate` / `highlight_prev_candidate` | walk the completion highlight without touching the text |

All three cuts put what they removed on the system clipboard, so a cut can be
pasted back. In a completing field they cut the text being typed and leave the
items already picked alone.

## Calendar mode

| command | what it does |
| --- | --- |
| `calendar_prev_day` / `calendar_next_day` | move the highlight one day |
| `calendar_prev_month` / `calendar_next_month` | flip months |
| `calendar_today` | jump back to today |
| `calendar_commit` | pick the highlighted day and close |
| `calendar_close` | close without picking |
| `calendar_clear` | empty the field |
| `calendar_toggle_grid` | put the grid away, freeing `h`/`l`/`j`/`k`/`t`/`d` to be typed |
| `calendar_jump_to_start` / `calendar_jump_to_end` | move the caret to a range's two ends |

## Task mode

| command | what it does |
| --- | --- |
| `toggle_completed_filter` | show or hide completed tasks |
| `toggle_subtask_visibility` | show or hide subtasks |
| `toggle_project_grouping` | group rows under project headings |
| `toggle_section_grouping` | group rows under section headings |
| `toggle_column_sort` | sort by the column under the cursor: descending, ascending, unsorted |
| `move_section_up` / `move_section_down` | move the cursor by section |
| `move_project_up` / `move_project_down` | move the cursor by project |
| `task_column_prev` / `task_column_next` | move the column cursor |
| `edit_column` | open the cell under the column cursor |
| `toggle_task_completed` | mark the task — or the selection — done |
| `open` | open the task under the cursor in Asana |
| `toggle_task_selection` | select or deselect the task under the cursor |
| `select_all_visible_tasks` | select every task currently listed |
| `invert_task_selection` | select what was not selected, and vice versa |
| `clear_task_selection` | select nothing |
| `clear_hidden_task_selection` | deselect only the tasks the current filters hide |
| `copy_tasks_to_clipboard` | copy the selected tasks to the system clipboard, as a Markdown checklist |
| `toggle_recent_pane` | show or hide the recently-edited pane |
| `set_edit_mode` | enter edit mode |
| `set_gantt_mode` | draw the Gantt chart and enter gantt mode |

## Column edit mode

A table cell is open.

| command | what it does |
| --- | --- |
| `commit_column_edit` | send the edit |
| `cancel_column_edit` | throw the edit away |
| `column_edit_next_value` / `column_edit_prev_value` | step a value picker through its options |
| `column_edit_clear` | clear the cell |

It also accepts every caret, cut, and completion command listed under
[filter edit mode](#filter-edit-mode) — the cell editor and the filter panel
share one text editor.

## Edit mode

The keys that change which rows exist.

| command | what it does |
| --- | --- |
| `insert_task` | a new task beside the one under the cursor |
| `insert_subtask` | a new subtask of the one under the cursor |
| `insert_section` | a new section after the cursor's section |
| `delete_section` | delete the first empty section of the cursor row's project |
| `move_task_to_next_section` / `move_task_to_prev_section` | move the task between its project's sections |
| `mark_for_deletion` | mark the task, or the selection, for deletion |
| `delete_marked_tasks` | carry out the marked deletions |
| `edit_cancel` | clear the marks, or leave edit mode when there are none |
| `toggle_task_selection` | select or deselect, as in task mode |

## Gantt mode

| command | what it does |
| --- | --- |
| `gantt_scroll_left` / `gantt_scroll_right` | scroll the timeline by a quarter window |
| `gantt_zoom_in` / `gantt_zoom_out` | step through the zoom ladder |
| `gantt_zoom_fit` | fit the window to whatever tasks are loaded |
| `gantt_today` | bring today to the left of the window |
| `gantt_add_column` / `gantt_remove_column` | show more or fewer table columns beside the chart |
| `cycle_gantt_color_key` | colour the bars by the next dimension |
| `gantt_open_order` | open the colour dialog |
| `toggle_gantt` | hide the chart |

## Gantt order mode

The colour dialog.

| command | what it does |
| --- | --- |
| `gantt_order_move_up` / `gantt_order_move_down` | move the selected value up or down the order |
| `gantt_order_move_top` / `gantt_order_move_bottom` | send it to either end |
| `gantt_order_commit` | save the order to `[gantt]` and close |
| `gantt_order_cancel` | close, writing nothing |
| `cycle_gantt_color_key` | switch dimension and rebuild the list |

`move_up` and `move_down` move the dialog's cursor here, which is why the
value itself moves on `ctrl-k` and `ctrl-j`.
