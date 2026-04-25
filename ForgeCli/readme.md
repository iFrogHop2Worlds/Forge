## ForgeCli Usage

`ForgeCli` supports a hybrid model:
- With arguments: one-shot Clap command mode.
- Without a subcommand: interactive REPL mode.

## One-shot commands (Clap)

Default DB path is `./forge_data` unless overridden with `--db-path`.

Examples:
- `cargo run -p ForgeCli -- put mykey myvalue`
- `cargo run -p ForgeCli -- get mykey`
- `cargo run -p ForgeCli -- delete mykey`
- `cargo run -p ForgeCli -- sync`
- `cargo run -p ForgeCli -- --db-path ./forge_data/dev put k v`

Supported command forms:
- `forge [--db-path <path>] put <key> <value>`
- `forge [--db-path <path>] get <key>`
- `forge [--db-path <path>] delete <key>`
- `forge [--db-path <path>] sync`

## REPL mode

Start REPL:
- `cargo run -p ForgeCli`
- `cargo run -p ForgeCli -- --db-path ./forge_data/dev`

Prompt:
- `forge>`

REPL commands:
- `PUT <key> <value>`
- `GET <key>`
- `DEL <key>` or `DELETE <key>`
- `SYNC`
- `CUR` (shows current datastore path)
- `CONNECT <path>` (switch to an existing/new path)
- `NEW <name>` (creates/switches to `./forge_data/<name>`)
- `HELP`
- `EXIT` or `QUIT`
