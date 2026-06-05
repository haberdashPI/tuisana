# Tuisana

Tuisana is a terminal UI for reviewing Asana projects and tasks.

## Configuration

The program reads `tuisana.toml` from the repository root.

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

### Project visibility

The repeated `[[project]]` section lets you pin project-specific visibility preferences by Asana project GID.

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

### Key bindings

The `[[bind]]` section maps keyboard input to command names.
If you omit a command from your config, the built-in default binding for that command still applies.

Current commands include:

- `quit`
- `move_up`
- `move_down`
- `open`
- `refresh`
- `clear_search`
- `toggle_selection`
- `select_all_visible`
- `invert_selection`
- `clear_selection`
- `undo_selection`
- `redo_selection`
- `toggle_starred_selected`
- `toggle_hidden_selected`
- `toggle_hidden_group`
- `toggle_task_view`
- `toggle_task_mode`
- `toggle_only_selected`
- `start_search`
- `search_fuzzy`
- `search_substring`
- `search_regex`
- `page_up`
- `page_down`

Project view shortcuts include:

- `?` to toggle compact vs expanded hint display
- `j`/`down` to move down
- `k`/`up` to move up
- `ctrl-u` and `ctrl-d` to page through the list
- `home` and `end` to jump to the top or bottom
- `space` to toggle selection for the current project
- `a` to select all visible projects
- `i` to invert the visible selection
- `c` to clear the selection
- `u` to undo the last selection change
- `ctrl-y` to redo the last selection change
- `*` to toggle starred state for the current selection
- `h` to toggle hidden state for the current selection
- `v` to show or hide hidden projects
- `t` to toggle the task panel
- `m` to switch task/project focus mode
- `o` to filter to selected projects only
- `/` to start search entry
- `ctrl-l` to clear the search string
- `ctrl-f`, `ctrl-s`, `ctrl-r` to switch search mode

When the task panel is visible, the screen is split so the project list keeps just the top part of the view and the task table gets the remaining space.

Search entry accepts ordinary typing, `backspace`, `enter`, and `esc`.
The status area now shows a dedicated search line so you can see whether the view is not searching, awaiting input, or filtering by a query.
The compact hint line always starts with `?`, so the expanded help toggle is always discoverable.

Example:

```toml
[[bind]]
key = "j"
command = "move_down"
```

## Running

Use the project tasks:

```bash
mise run
mise test
mise coverage
```

## Notes

The browser-based Asana login flow is planned for a later milestone. For now, the app expects a PAT in config.
