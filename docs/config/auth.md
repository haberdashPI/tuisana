# Authentication

```toml
[auth]
personal_access_token = "1/1234567890:abcdef..."
workspace_gid = "1200000000000001"   # optional
```

`[auth]` is the one section tuisana cannot start without.

## `personal_access_token`

**Required.** A personal access token is the quickest way to authenticate with
Asana today.

1. Open the [Asana developer console](https://app.asana.com/0/my-apps).
2. Create a personal access token.
3. Copy it into `personal_access_token`.

Asana documents
[token creation](https://developers.asana.com/docs/personal-access-token) and
[bearer-token usage](https://developers.asana.com/docs/authentication) in more
detail.

::: danger Keep this secret
The token carries your full Asana access. `tuisana.toml` is listed in this
repository's `.gitignore`, as are the `tuisana.backup*.toml` files the config
migration writes — those are byte-for-byte copies and carry the same token.
:::

## `workspace_gid`

**Optional.** Omit it and tuisana lists every project your token can see,
without narrowing to one workspace.

Set it to restrict the project list to a single workspace:

1. Call `GET /workspaces` with your token, or use Asana's API explorer.
2. Find the workspace you want.
3. Copy its `gid`.

Asana's reference:
[list workspaces](https://developers.asana.com/reference/getworkspaces),
[get a workspace](https://developers.asana.com/reference/getworkspace).

### The "assigned to me" row

The project list carries a **`No Project (Assigned to Me)`** row, which appears
automatically once tuisana can resolve your logged-in user. `workspace_gid` is
what lets tuisana *load* that row's tasks after you select it — without it, the
row appears but cannot be fetched.

Only tasks outside every project stay under that row's name. A task that does
live in a project — for a subtask, the project of the nearest parent that has
one — is grouped under that project instead, and shown only when the project is
selected too.
