# tuisana

A terminal UI for reviewing and editing Asana projects and tasks the way power
users like to work: in the terminal with a keyboard shortcut for everything.

**📖 [Documentation](https://haberdashpi.github.io/tuisana/)**

> [!WARNING]
> This is a personal "vibe coded" project, built as an experiment to understand
> how well that actually works. I have been dogfooding its use for personal task
> management. This has been going perfectly well for my own use case, but YMMV.

## Install

```sh
git clone https://github.com/haberdashPI/tuisana.git
cd tuisana
cargo install --path .
```

## Set up

tuisana reads **`tuisana.toml` from the directory you run it in**. A minimal
file is three lines plus a token:

```toml
[header]
type = "tuisana"
version = 3.0

[auth]
personal_access_token = "1/1234567890:abcdef..."
```

Create the token in the [Asana developer console](https://app.asana.com/0/my-apps)
(Asana's [docs](https://developers.asana.com/docs/personal-access-token)), then
paste it in. Keep it secret — `tuisana.toml` is already gitignored.

Everything else has a default. For a complete, commented starting point, copy
[`tuisana.toml.example`](./tuisana.toml.example) to `tuisana.toml`.

## Run

```sh
tuisana
```

A session opens in the project list.

| key | what it does |
| --- | --- |
| `j` / `k` | move down and up |
| `space` | **select** the project under the cursor |
| `t` | show the task table |
| `f` | open the filter panel |
| `?` | help for the current mode |
| `q` | quit |

The rule worth knowing up front: **nothing is loaded until you select it.** A
project merely under the cursor is not fetched, and the filter panel will not
open while nothing is selected.

From there, `?` is the map — it lists the keys bound in whatever mode you are
standing in, built from your own config.

## Configure

Full reference at
**[haberdashpi.github.io/tuisana](https://haberdashpi.github.io/tuisana/)**:

- [Authentication](https://haberdashpi.github.io/tuisana/config/auth) — your token, and optionally a workspace
- [Appearance](https://haberdashpi.github.io/tuisana/config/appearance) — colours and glyphs
- [Key bindings](https://haberdashpi.github.io/tuisana/config/keybindings) — syntax, modes, and all 200-odd defaults
- [Commands](https://haberdashpi.github.io/tuisana/reference/commands) — every bindable command name
- [Settings tuisana writes](https://haberdashpi.github.io/tuisana/config/managed) — the sections the app maintains for you

## Develop

The repository uses [mise](https://mise.jdx.dev/):

```sh
cargo run       # run from source
mise test       # cargo test
mise coverage   # cargo llvm-cov
```

The docs site is VitePress, and lives in [`docs/`](./docs):

```sh
npm install
npm run docs:dev
```

## License

[MIT](./LICENSE.md)
