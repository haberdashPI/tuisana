# Getting started

## Install

tuisana is a Rust program. Build and install it with Cargo:

```sh
git clone https://github.com/haberdashPI/tuisana.git
cd tuisana
cargo install --path .
```

## Create a config file

tuisana reads **`tuisana.toml` from the directory you run it in**. There is no
search of your home directory and no `--config` flag, so keep the file beside
whatever you treat as your workspace and run `tuisana` from there.

A minimal file is three lines:

```toml
[header]
type = "tuisana"
version = 3.0

[auth]
personal_access_token = "1/1234567890:abcdef..."
```

Everything else has a default. The repository ships a fully commented
[`tuisana.toml.example`](https://github.com/haberdashPI/tuisana/blob/main/tuisana.toml.example)
if you would rather start from a complete file.

## Get a personal access token

1. Open the [Asana developer console](https://app.asana.com/0/my-apps).
2. Create a personal access token.
3. Paste it into `auth.personal_access_token`.

Keep that value secret — it is a credential with your full Asana access.
`tuisana.toml` is already in this repository's `.gitignore` for that reason.

See [Authentication](/config/auth) for the optional `workspace_gid` setting and
what it changes.

## Run it

```sh
tuisana
```

## The first five minutes

A session opens in the project list.

| key | what it does |
| --- | --- |
| `j` / `k` | move down and up — everywhere in the app |
| `space` | **select** the project under the cursor |
| `t` | show the task table for the selected projects |
| `f` | open the filter panel |
| `?` | help for the current mode, built from your own key bindings |
| `q` | quit |

The one rule worth knowing up front: **nothing is loaded until you select it.**
A project merely under the cursor is not fetched, and the filter panel refuses
to open while nothing is selected — a filter with no projects is a question
with no subject.

From there, `?` is the map. It lists the keys bound in the mode you are
standing in, so it stays correct even after you rebind things.

## Development tasks

The repository uses [mise](https://mise.jdx.dev/) for its task runner:

```sh
cargo run       # run from source
mise test       # cargo test
mise coverage   # cargo llvm-cov
```
