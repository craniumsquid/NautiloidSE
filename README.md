# Nautiloid Save Editor

A desktop save editor for **Baldur's Gate 3** (Patch 8) on Windows, macOS and Linux, including
Steam Deck. It is at an early stage: the code that reads and writes saves exists and is tested;
the editor's interface does not exist yet, and nothing has been tried in the game.

- [SCOPE.md](SCOPE.md): what the editor will and won't do, the build order and where it stands.
- [docs/FORMAT.md](docs/FORMAT.md): what is known about the save format.

## Looking inside a save

`nautiloid-cli` is a command-line tool for working out the format. A save is a `.lsv` file or
the folder that holds one.

```bash
cargo run --release --bin nautiloid-cli -- info "path/to/save folder"
```

Run it without arguments for the other commands: `list`, `dump`, `find`, `diff`, `party`,
`tables`, `rows`, `check`, `extract`, `rewrite` and `set`.

## Tests

```bash
cargo test
```

Real saves are private and are not in the repository. To run the checks against your own, point
`NAUTILOID_SAVES` at a folder of save folders:

```bash
NAUTILOID_SAVES="path/to/Savegames/Story" cargo test
```

Unofficial. Not affiliated with or endorsed by Larian Studios.

Licensed under GPL-3.0-only; see [LICENSE](LICENSE).
