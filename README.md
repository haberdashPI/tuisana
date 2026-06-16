# Tuisana

Tuisana is a terminal UI for reviewing Asana projects and tasks the way "power users" like me like to work with apps: with keyboard shortcuts for everything.

> [!WARNING]
>
> This is a early-stage project largely developed with/by AI. I have personally reviewed all
> of these files, but all code was written by Codex or Claude Code. Documentation is not yet
> optimized for users at this point. This is not a project I have the time/resources to
> manually develop by hand that I thought might be a plausible, well-constrained target for
> an LLM to write. Later milestones may lead to edit abilities, but for now I have focused
> on read-only features out of an abundance of caution.

## Installation

This is a rust app, you can install it like this:

```sh
cargo install --path .
```

## Configuration

The program reads `tuisana.toml` from the current directory.

Start from [`tuisana.toml.example`](./tuisana.toml.example) and copy it to `tuisana.toml`, then fill in the auth values and any project visibility preferences.

### Fixed header values

Leave these unchanged:

- `header.type = "tuisana"`
- `header.version = 1.0`

These values identify the config format and are validated by the app.

### `auth.personal_access_token`

This is the quickest way to authenticate with Asana today.

How to get it:

1. Open the Asana developer console.
2. Create a personal access token.
3. Copy the token into `auth.personal_access_token`.

Asana documents PAT creation and bearer-token usage here:

- https://developers.asana.com/docs/personal-access-token
- https://developers.asana.com/docs/authentication

Keep this value secret.

### `auth.workspace_gid`

This field is optional. Omit it if you want Tuisana to list projects visible to your token without narrowing to one workspace.

If you want to restrict project listing to a specific workspace, you need that workspace's GID.

How to find it:

1. Use Asana's API explorer or call `GET /workspaces` with your token.
2. Find the workspace you want.
3. Copy its `gid` into `auth.workspace_gid`.

Asana docs:

- https://developers.asana.com/reference/getworkspaces
- https://developers.asana.com/reference/getworkspace

### Key bindings

The `[[bind]]` section maps keyboard input to command names.
If you omit a command from your config, the built-in default binding for that command still applies.

Each binding may include an optional `mode` field to restrict it to a specific UI context.
Available modes are `any` (default), `project`, `project_search`, `filter`, `filter_edit`, and `task`.

Example:

```toml
[[bind]]
key = "j"
command = "move_down"

[[bind]]
key = "ctrl-j"
mode = "filter_edit"
command = "filter_done_editing"
```

All bindable commands:

**Global (any mode)**

- `quit`
- `move_up`
- `move_down`
- `page_up`
- `page_down`
- `jump_top`
- `jump_bottom`
- `scroll_left`
- `scroll_right`
- `refresh`
- `toggle_help_details`
- `toggle_task_view`
- `toggle_task_mode`
- `set_project_mode`
- `set_filter_mode`
- `set_task_mode`
- `resize_top_pane_up`
- `resize_top_pane_down`
- `minimize_top_pane`
- `maximize_top_pane`
- `restore_top_pane`

**Project mode**

- `open`
- `start_search`
- `toggle_selection`
- `select_all_visible`
- `select_all_starred_visible`
- `select_all_non_hidden_visible`
- `invert_selection`
- `clear_selection`
- `undo_selection`
- `redo_selection`
- `toggle_starred_selected`
- `toggle_hidden_selected`
- `toggle_hidden_group`
- `toggle_only_selected`
- `search_fuzzy`
- `search_substring`
- `search_regex`

**Project search mode**

- `clear_search`

**Filter mode**

- `begin_filter_edit`
- `toggle_task_filters`
- `cycle_filter_string_mode`
- `clear_search`
- `search_fuzzy`
- `search_substring`
- `search_regex`

**Filter edit mode**

- `filter_done_editing`
- `filter_cancel_editing`
- `filter_move_label_left`
- `filter_move_label_right`
- `filter_cycle_label_up`
- `filter_cycle_label_down`
- `filter_add_label`
- `filter_delete_label`
- `clear_search`
- `search_fuzzy`
- `search_substring`
- `search_regex`

**Task mode**

- `toggle_completed_filter`
- `toggle_subtask_visibility`
- `toggle_project_grouping`
- `toggle_section_grouping`
- `cycle_task_sort`
- `move_section_up`
- `move_section_down`
- `move_project_up`
- `move_project_down`

---

The top window is shared between the project list and the filter view. When the task panel is visible, the screen is split so the top window keeps the project or filter view and the task table gets the remaining space.

**Global shortcuts** (active in all modes unless overridden):

- `?` to toggle compact vs expanded hint display
- `j`/`down` to move down, `k`/`up` to move up
- `ctrl-u` and `ctrl-d` to page through the list
- `home` and `end` to jump to the top or bottom
- `left` and `right` to scroll columns
- `t` to toggle the task panel, `m` to toggle task mode
- `f` to switch to filter mode, `p` to switch to project mode
- `[` and `]` to shrink or grow the top window
- `{` to minimize the top window, `}` to maximize it, `0` to restore
- `r` to refresh

**Project view shortcuts**:

- `enter` to open the selected project in Asana
- `space` to toggle selection for the current project
- `a` to select all visible projects
- `i` to invert the visible selection
- `c` to clear the selection
- `u` to undo the last selection change, `ctrl-y` to redo
- `*` to toggle starred state for the current selection
- `h` to toggle hidden state for the current selection
- `v` to show or hide hidden projects
- `!` to select all starred visible projects
- `@` to select all visible non-hidden projects
- `o` to filter to selected projects only
- `/` to start search entry
- `ctrl-f`, `ctrl-s`, `ctrl-r` to switch search mode (fuzzy / substring / regex)

**Project search** accepts ordinary typing, `backspace`, `enter`, and `esc`.
`ctrl-l` clears the search string.

**Filter panel shortcuts** (filter browse mode):

- `j`/`k` to move between filter fields
- `enter` to start editing the selected field
- `esc` to close the panel and return to task mode
- `f` to toggle the filter panel
- `s` to cycle the string-match mode
- `ctrl-l` to clear the search string
- `ctrl-f`, `ctrl-s`, `ctrl-r` to switch search mode

**Filter field editing** (filter edit mode):

- Ordinary typing to edit a text or date field
- `backspace` to delete the last character
- `enter` to confirm the edit and return to filter browse mode
- `esc` to discard the edit, close the panel, and return to task mode
- `ctrl-l`, `ctrl-f`, `ctrl-s`, `ctrl-r` as above

For fields include a fixed set of labels, the edit keys navigate instead of typing:

- `h`/`l` to move between the set of listed labeles
- `j`/`k` to cycle the selected label between the possible options
- `a` to add a label, `d` to delete the current one

**Task view shortcuts**:

- `c` to toggle the completed filter
- `z` to toggle subtask visibility
- `,` to toggle project grouping, `.` to toggle section grouping
- `s` to cycle the task sort
- `[` and `]` to move by section
- `{` and `}` to move by project

### Project visibility

The repeated `[[project]]` section lets you pin project-specific visibility preferences by Asana project GID. It is usually updated when interacting with the app to change project visibility and project stars, rather than modified directly by a user.

Each entry supports:

- `gid`
- `starred`
- `hidden`

How it works:

- `starred = true` sorts the project ahead of unstarred projects.
- `hidden = true` keeps the project in the hidden group.
- Hidden projects stay available in the list, but they are shown after the visible projects only when you toggle them on.
- Hidden projects are marked explicitly in the UI so they are easy to spot.

The default toggle for hidden projects is `v`.

Example:

```toml
[[project]]
gid = "123"
starred = true
hidden = false

[[project]]
gid = "456"
starred = false
hidden = true
```
## Running

Use the project tasks:

```bash
cargo run
mise test
mise coverage
```
