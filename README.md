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

The default toggle for hidden projects is `h`.

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

Current commands include:

- `quit`
- `move_up`
- `move_down`
- `open`
- `refresh`
- `page_up`
- `page_down`
- `toggle_hidden` (bound to `h`)

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
