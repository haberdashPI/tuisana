# Tuisana

Tuisana is a terminal UI for reviewing Asana projects and tasks.

## Configuration

The program reads `tuisana.toml` from the repository root.

Start from [`tuisana.toml.example`](./tuisana.toml.example) and copy it to `tuisana.toml`, then fill in the auth values.

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

Current commands include:

- `quit`
- `move_up`
- `move_down`
- `open`
- `refresh`
- `page_up`
- `page_down`

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
