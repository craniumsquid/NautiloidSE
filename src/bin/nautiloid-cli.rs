//! Command-line tool for looking inside saves. It is for working out the format, not for
//! players: the editor itself has no need of it.

use anyhow::{anyhow, bail, ensure, Context, Result};
use nautiloid_save_editor::diff;
use nautiloid_save_editor::lsf::{Lsf, NodeId, Sections};
use nautiloid_save_editor::lspk::Method;
use nautiloid_save_editor::newage::NewAge;
use nautiloid_save_editor::save::{self, Save, Support};
use nautiloid_save_editor::value;
use std::collections::BTreeMap;
use std::io::{self, BufWriter, Write};
use std::path::Path;

const USAGE: &str = "\
nautiloid-cli: look inside Baldur's Gate 3 saves

A <save> is a .lsv file or the save folder that holds one.

  list <save>
      The files in the save package.
  info <save>
      Game version, party, difficulty and mods.
  dump <save> <file.lsf> [--node ID | --path PATH] [--depth N]
      A document as text. Nodes are [Name] #id; attributes are Name: Type = value.
  find <save> [--value TEXT] [--attr NAME] [--in NODE] [--file FILE] [--limit N]
      Attributes whose value is TEXT (numbers) or contains it (text and UUIDs), whose name
      is NAME, or that sit on a node called NODE. At least one of the three.
  diff <save> <save> [--file FILE] [--grep TEXT] [--limit N] [--summary]
      What differs between two saves, document by document.
  party <save>
      The party as the entity data has it: level, experience and ability scores.
  tables <save> <file.lsf>
      The tables in a document's entity data (its NewAge blob).
  rows <save> <file.lsf> <table> [--limit N]
      A table's rows as 32-bit numbers, with the entity each belongs to.
  set-xp <save> <out.lsv> <amount>
      Write a copy in which every party member's experience is <amount>. For testing that
      the game accepts an edit to the entity data.
  check <save>...
      Read each save, write it back in memory and compare.
  extract <save> <file> <out>
      Write one file's uncompressed contents to <out>.
  rewrite <save> <out.lsv> [--reencode]
      Write a copy of the package. With --reencode every document is parsed, written and
      compressed again, to test that the game accepts this tool's output.
";

/// Arguments after the command: positional ones, and `--name value` or bare `--name` flags.
struct Args {
    positional: Vec<String>,
    options: BTreeMap<String, String>,
}

impl Args {
    fn parse(raw: &[String], flags: &[&str], valued: &[&str]) -> Result<Args> {
        let mut args = Args {
            positional: Vec::new(),
            options: BTreeMap::new(),
        };
        let mut iter = raw.iter();
        while let Some(arg) = iter.next() {
            if let Some(name) = arg.strip_prefix("--") {
                if flags.contains(&name) {
                    args.options.insert(name.to_owned(), String::new());
                } else if valued.contains(&name) {
                    let value = iter
                        .next()
                        .ok_or_else(|| anyhow!("--{name} needs a value"))?;
                    args.options.insert(name.to_owned(), value.clone());
                } else {
                    bail!("unknown option --{name}");
                }
            } else {
                args.positional.push(arg.clone());
            }
        }
        Ok(args)
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.options.get(name).map(String::as_str)
    }

    fn number(&self, name: &str) -> Result<Option<usize>> {
        self.get(name)
            .map(|v| v.parse().with_context(|| format!("--{name} {v}")))
            .transpose()
    }

    fn expect(&self, count: usize, usage: &str) -> Result<()> {
        ensure!(
            self.positional.len() == count,
            "usage: nautiloid-cli {usage}"
        );
        Ok(())
    }
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let result = run(&raw);
    if let Err(error) = result {
        // A closed pipe (`| head`) is not a failure.
        if error
            .downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
        {
            return;
        }
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run(raw: &[String]) -> Result<()> {
    let Some((command, rest)) = raw.split_first() else {
        print!("{USAGE}");
        return Ok(());
    };
    let out = &mut BufWriter::new(io::stdout().lock());
    match command.as_str() {
        "list" => list(&Args::parse(rest, &[], &[])?, out),
        "info" => info(&Args::parse(rest, &[], &[])?, out),
        "dump" => dump(&Args::parse(rest, &[], &["node", "path", "depth"])?, out),
        "find" => find(
            &Args::parse(rest, &[], &["value", "attr", "in", "file", "limit"])?,
            out,
        ),
        "diff" => compare(
            &Args::parse(rest, &["summary"], &["file", "grep", "limit"])?,
            out,
        ),
        "party" => party(&Args::parse(rest, &[], &[])?, out),
        "tables" => tables(&Args::parse(rest, &[], &[])?, out),
        "rows" => rows(&Args::parse(rest, &[], &["limit"])?, out),
        "set-xp" => set_xp(&Args::parse(rest, &[], &[])?, out),
        "check" => check(&Args::parse(rest, &[], &[])?, out),
        "extract" => extract(&Args::parse(rest, &[], &[])?),
        "rewrite" => rewrite(&Args::parse(rest, &["reencode"], &[])?, out),
        "help" | "--help" | "-h" => {
            write!(out, "{USAGE}")?;
            Ok(())
        }
        other => bail!("unknown command {other}; run without arguments for help"),
    }?;
    out.flush()?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn list(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(1, "list <save>")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let package = &save.package;
    writeln!(out, "{}", save.path.display())?;
    writeln!(out, "{:>12} {:>12}  {:<5} name", "stored", "size", "how")?;
    for entry in package.entries_on_disk() {
        writeln!(
            out,
            "{:>12} {:>12}  {:<5} {}",
            entry.stored_len(),
            entry.len(),
            entry.method().label(),
            entry.name()
        )?;
    }
    let computed = package.compute_md5()?;
    writeln!(
        out,
        "header checksum {} ({})",
        hex(&package.stored_md5()),
        if computed == package.stored_md5() {
            "matches the contents"
        } else {
            "DOES NOT match the contents"
        }
    )?;
    Ok(())
}

fn info(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(1, "info <save>")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let info = save.info()?;
    let header = save.meta_header()?;
    writeln!(out, "{}", save.path.display())?;
    writeln!(out, "save name     {}", info.save_name)?;
    writeln!(
        out,
        "game version  {} (engine {})",
        info.game_version,
        header.engine_version_text()
    )?;
    writeln!(out, "platform      {}", info.platform)?;
    writeln!(out, "level         {}", info.current_level)?;
    writeln!(
        out,
        "difficulty    {}",
        if info.difficulty.is_empty() {
            "(not in SaveInfo.json)".to_owned()
        } else {
            info.difficulty.join(", ")
        }
    )?;
    match save.support() {
        Support::Editable => writeln!(out, "editable      yes")?,
        Support::ReadOnly(why) => writeln!(out, "editable      no: {why}")?,
    }
    writeln!(out, "party")?;
    for member in &info.party {
        let classes: Vec<String> = member
            .classes
            .iter()
            .map(|c| {
                if c.sub.is_empty() {
                    c.main.clone()
                } else {
                    format!("{} ({})", c.main, c.sub)
                }
            })
            .collect();
        writeln!(
            out,
            "  {:<14} level {:<2} {:<18} {}  xp {} ({} into the level)",
            member.origin,
            member.level,
            member.race,
            classes.join(" / "),
            member.experience_total,
            member.experience_in_level
        )?;
    }
    let mods = save::mods(&save.lsf(save::META)?)?;
    writeln!(out, "mods ({})", mods.len())?;
    for entry in &mods {
        writeln!(out, "  {:<44} {}", entry.name, entry.folder)?;
    }
    Ok(())
}

/// Resolves a path as `Lsf::path` prints it: names separated by `/`, with an optional `[n]`
/// picking the n-th sibling of that name.
fn resolve(lsf: &Lsf, path: &str) -> Result<NodeId> {
    let mut current: Option<NodeId> = None;
    for part in path.split('/').filter(|p| !p.is_empty()) {
        let (name, index) = match part.strip_suffix(']').and_then(|p| p.split_once('[')) {
            Some((name, index)) => (
                name,
                index.parse::<usize>().with_context(|| part.to_owned())?,
            ),
            None => (part, 0),
        };
        let siblings = match current {
            Some(node) => lsf.children(node),
            None => lsf.roots(),
        };
        current = Some(
            siblings
                .iter()
                .copied()
                .filter(|&n| lsf.node_name(n) == name)
                .nth(index)
                .ok_or_else(|| anyhow!("no node {part} in {path}"))?,
        );
    }
    current.ok_or_else(|| anyhow!("empty path"))
}

fn dump_node(
    lsf: &Lsf,
    node: NodeId,
    indent: usize,
    depth: Option<usize>,
    out: &mut impl Write,
) -> Result<()> {
    let pad = "  ".repeat(indent);
    let key = lsf
        .key_attribute(node)
        .map(|k| format!(" key={k}"))
        .unwrap_or_default();
    writeln!(out, "{pad}[{}] #{node}{key}", lsf.node_name(node))?;
    for &attr in lsf.attrs(node) {
        writeln!(
            out,
            "{pad}  {}: {} = {}",
            lsf.attr_name(attr),
            value::type_name(lsf.attr_type(attr)),
            diff::show(lsf, attr)
        )?;
    }
    let children = lsf.children(node);
    match depth {
        Some(0) if !children.is_empty() => {
            writeln!(out, "{pad}  … {} children", children.len())?;
        }
        _ => {
            for &child in children {
                dump_node(
                    lsf,
                    child,
                    indent + 1,
                    depth.map(|d| d.saturating_sub(1)),
                    out,
                )?;
            }
        }
    }
    Ok(())
}

fn dump(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(
        2,
        "dump <save> <file.lsf> [--node ID | --path PATH] [--depth N]",
    )?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let lsf = save.lsf(&args.positional[1])?;
    let depth = args.number("depth")?;
    let start = match (args.number("node")?, args.get("path")) {
        (Some(id), _) => {
            ensure!(
                id < lsf.node_count(),
                "the document has {} nodes",
                lsf.node_count()
            );
            Some(id as NodeId)
        }
        (None, Some(path)) => Some(resolve(&lsf, path)?),
        (None, None) => None,
    };
    match start {
        Some(node) => {
            writeln!(out, "# {}", lsf.path(node))?;
            dump_node(&lsf, node, 0, depth, out)?;
        }
        None => {
            for &root in lsf.roots() {
                dump_node(&lsf, root, 0, depth, out)?;
            }
        }
    }
    Ok(())
}

fn find(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(
        1,
        "find <save> [--value TEXT] [--attr NAME] [--in NODE] [--file FILE]",
    )?;
    let wanted_value = args.get("value");
    let wanted_attr = args.get("attr");
    let wanted_node = args.get("in");
    ensure!(
        wanted_value.is_some() || wanted_attr.is_some() || wanted_node.is_some(),
        "give at least one of --value, --attr and --in"
    );
    let limit = args.number("limit")?.unwrap_or(100);
    let save = Save::open(Path::new(&args.positional[0]))?;
    let lowered = wanted_value.map(str::to_lowercase);
    let mut shown = 0usize;
    let mut total = 0usize;
    for name in save.lsf_names() {
        if args.get("file").is_some_and(|f| f != name) {
            continue;
        }
        let lsf = save.lsf(name)?;
        for attr in 0..lsf.attr_count() as u32 {
            if wanted_attr.is_some_and(|a| a != lsf.attr_name(attr)) {
                continue;
            }
            let node = lsf.attr_node(attr);
            if wanted_node.is_some_and(|n| n != lsf.node_name(node)) {
                continue;
            }
            if let Some(wanted) = &lowered {
                let Ok(decoded) = lsf.value(attr) else {
                    continue;
                };
                let matches = match &decoded {
                    value::Value::Text(_)
                    | value::Value::Translated { .. }
                    | value::Value::Uuid(_) => {
                        decoded.to_string().to_lowercase().contains(wanted.as_str())
                    }
                    value::Value::Bytes(_) => false,
                    other => other.to_string() == *wanted,
                };
                if !matches {
                    continue;
                }
            }
            total += 1;
            if shown < limit {
                shown += 1;
                writeln!(
                    out,
                    "{name} #{node} {} : {} ({}) = {}",
                    lsf.path(node),
                    lsf.attr_name(attr),
                    value::type_name(lsf.attr_type(attr)),
                    diff::show(&lsf, attr)
                )?;
            }
        }
    }
    writeln!(
        out,
        "{total} found{}",
        if total > shown {
            format!(", {shown} shown")
        } else {
            String::new()
        }
    )?;
    Ok(())
}

fn compare(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(
        2,
        "diff <save> <save> [--file FILE] [--grep TEXT] [--limit N] [--summary]",
    )?;
    let a = Save::open(Path::new(&args.positional[0]))?;
    let b = Save::open(Path::new(&args.positional[1]))?;
    let limit = args.number("limit")?.unwrap_or(200);
    let grep = args.get("grep").map(str::to_lowercase);
    let summary_only = args.get("summary").is_some();

    for entry in a.package.entries() {
        let name = entry.name();
        if args.get("file").is_some_and(|f| f != name) {
            continue;
        }
        let Some(other) = b.package.find(name) else {
            writeln!(out, "== {name}: only in the first save")?;
            continue;
        };
        let (old, new) = (a.package.unpack(entry)?, b.package.unpack(other)?);
        if !name.ends_with(".lsf") {
            writeln!(
                out,
                "== {name}: {}",
                if old == new {
                    "identical"
                } else {
                    "differs (not a document; not compared further)"
                }
            )?;
            continue;
        }
        let (old, new) = (Lsf::parse(&old)?, Lsf::parse(&new)?);
        let mut changes = diff::diff(&old, &new);
        if let Some(grep) = &grep {
            changes.retain(|c| c.to_string().to_lowercase().contains(grep.as_str()));
        }
        writeln!(
            out,
            "== {name}: {} changes ({} -> {} nodes)",
            changes.len(),
            old.node_count(),
            new.node_count()
        )?;
        if !summary_only {
            for change in changes.iter().take(limit) {
                writeln!(out, "{change}")?;
            }
            if changes.len() > limit {
                writeln!(
                    out,
                    "… {} more (raise --limit to see them)",
                    changes.len() - limit
                )?;
            }
        }
        if summary_only || changes.len() > limit {
            let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
            for change in &changes {
                *kinds.entry(change.kind()).or_default() += 1;
            }
            let mut kinds: Vec<(String, usize)> = kinds.into_iter().collect();
            kinds.sort_by(|x, y| y.1.cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
            writeln!(out, "-- by kind")?;
            for (kind, count) in kinds
                .iter()
                .take(if summary_only { usize::MAX } else { 60 })
            {
                writeln!(out, "{count:>7}  {kind}")?;
            }
        }
        if let (Ok(before), Ok(after)) = (NewAge::from_lsf(&old), NewAge::from_lsf(&new)) {
            writeln!(out, "-- entity data in {name}")?;
            compare_entities(&before, &after, limit, out)?;
        }
    }
    for entry in b.package.entries() {
        if !a.package.contains(entry.name()) && args.get("file").is_none_or(|f| f == entry.name()) {
            writeln!(out, "== {}: only in the second save", entry.name())?;
        }
    }
    Ok(())
}

fn party(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(1, "party <save>")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let globals = save.lsf(save::GLOBALS)?;
    let entities = NewAge::from_lsf(&globals)?;
    writeln!(
        out,
        "{:<36}  {:<36}  level  experience  Str Dex Con Int Wis Cha",
        "entity", "template"
    )?;
    for c in save::party(&globals, &entities)? {
        let abilities: Vec<String> = c.abilities.iter().map(|a| format!("{a:>3}")).collect();
        writeln!(
            out,
            "{}  {:<36}  {:>5}  {:>10}  {}",
            c.entity,
            c.template,
            c.level,
            c.experience,
            abilities.join(" ")
        )?;
    }
    if let Some(table) = entities.table(save::fields::CAMP_SUPPLIES) {
        if entities.tables()[table].rows == 1 {
            let row = entities.row(table, 0);
            writeln!(
                out,
                "camp supplies {}",
                u32::from_le_bytes([row[0], row[1], row[2], row[3]])
            )?;
        }
    }
    Ok(())
}

fn tables(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(2, "tables <save> <file.lsf>")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let entities = NewAge::from_lsf(&save.lsf(&args.positional[1])?)?;
    writeln!(
        out,
        "{} entities, {} tables",
        entities.entity_count(),
        entities.tables().len()
    )?;
    writeln!(out, "{:>8} {:>7}  {:<9} name", "rows", "row len", "kind")?;
    for table in entities.tables() {
        writeln!(
            out,
            "{:>8} {:>7}  {:<9} {}",
            table.rows,
            table.row_len,
            if table.is_component() {
                "component"
            } else {
                "data"
            },
            table.name
        )?;
    }
    Ok(())
}

/// A row as text: 32-bit numbers, then any bytes left over.
fn row_text(row: &[u8]) -> String {
    let mut parts: Vec<String> = row
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]).to_string())
        .collect();
    let rest = row.chunks_exact(4).remainder();
    if !rest.is_empty() {
        parts.push(format!("0x{}", hex(rest)));
    }
    parts.join(" ")
}

fn rows(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(3, "rows <save> <file.lsf> <table> [--limit N]")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let entities = NewAge::from_lsf(&save.lsf(&args.positional[1])?)?;
    let name = &args.positional[2];
    let id = entities
        .table(name)
        .ok_or_else(|| anyhow!("no table called {name}; see the tables command"))?;
    let table = &entities.tables()[id];
    let limit = args.number("limit")?.unwrap_or(50);
    writeln!(
        out,
        "{name}: {} rows of {} bytes",
        table.rows, table.row_len
    )?;
    for row in 0..table.rows.min(limit) {
        let owner = entities
            .owner(id, row)
            .map(|e| entities.entity_id(e).to_string())
            .unwrap_or_default();
        writeln!(
            out,
            "{row:>6}  {owner}  {}",
            row_text(entities.row(id, row))
        )?;
    }
    if table.rows > limit {
        writeln!(
            out,
            "… {} more (raise --limit to see them)",
            table.rows - limit
        )?;
    }
    Ok(())
}

/// What differs between the entity data of two documents, table by table. Rows of a
/// component table are matched by the entity that owns them, since rows move.
fn compare_entities(a: &NewAge, b: &NewAge, limit: usize, out: &mut impl Write) -> Result<()> {
    let mut shown = 0;
    for (id_b, table_b) in b.tables().iter().enumerate() {
        let Some(id_a) = a.table(&table_b.name) else {
            writeln!(out, "   + table {} ({} rows)", table_b.name, table_b.rows)?;
            continue;
        };
        let table_a = &a.tables()[id_a];
        if table_a.row_len != table_b.row_len || !table_b.is_component() || !table_a.is_component()
        {
            if table_a.rows != table_b.rows {
                writeln!(
                    out,
                    "   {}: {} -> {} rows",
                    table_b.name, table_a.rows, table_b.rows
                )?;
            }
            continue;
        }
        let mut before = std::collections::HashMap::new();
        for row in 0..table_a.rows {
            if let Some(owner) = a.owner(id_a, row) {
                before.insert(a.entity_id(owner), row);
            }
        }
        let (mut changed, mut added) = (0, 0);
        let mut lines = Vec::new();
        for row in 0..table_b.rows {
            let Some(owner) = b.owner(id_b, row) else {
                continue;
            };
            let entity = b.entity_id(owner);
            match before.remove(&entity) {
                None => added += 1,
                Some(old) if a.row(id_a, old) != b.row(id_b, row) => {
                    changed += 1;
                    if lines.len() < 8 {
                        lines.push(format!(
                            "      {entity}  {}  ->  {}",
                            row_text(a.row(id_a, old)),
                            row_text(b.row(id_b, row))
                        ));
                    }
                }
                Some(_) => {}
            }
        }
        let removed = before.len();
        if changed + added + removed > 0 {
            writeln!(
                out,
                "   {}: {changed} changed, {added} added, {removed} removed (of {} rows)",
                table_b.name, table_b.rows
            )?;
            if shown < limit {
                for line in &lines {
                    writeln!(out, "{line}")?;
                }
                shown += lines.len();
            }
        }
    }
    for table in a.tables() {
        if b.table(&table.name).is_none() {
            writeln!(out, "   - table {} ({} rows)", table.name, table.rows)?;
        }
    }
    Ok(())
}

fn set_xp(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(3, "set-xp <save> <out.lsv> <amount>")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let target = Path::new(&args.positional[1]);
    let amount: u32 = args.positional[2].parse().context("amount")?;
    ensure!(!target.exists(), "{} already exists", target.display());
    if let Support::ReadOnly(why) = save.support() {
        bail!("{why}");
    }
    let mut globals = save.lsf(save::GLOBALS)?;
    let mut entities = NewAge::from_lsf(&globals)?;
    for c in save::party(&globals, &entities)? {
        let index = entities
            .entity_index(c.entity)
            .ok_or_else(|| anyhow!("party member {} is not in the entity table", c.entity))?;
        for offset in save::fields::EXPERIENCE_TOTAL {
            entities.set_u32(save::fields::EXPERIENCE, index, offset, amount)?;
        }
        writeln!(out, "{}: experience {} -> {amount}", c.entity, c.experience)?;
    }
    entities.store(&mut globals)?;
    let mut package = save.package.clone();
    package.replace(save::GLOBALS, &globals.write()?)?;
    let bytes = package.write()?;

    // Read the result back: the edit is there and nothing else in the entity data moved.
    let written = nautiloid_save_editor::lspk::Package::read(bytes.clone())?;
    ensure!(written.compute_md5()? == written.stored_md5(), "checksum");
    let globals_after = Lsf::parse(&written.read_file(save::GLOBALS)?)?;
    let entities_after = NewAge::from_lsf(&globals_after)?;
    for c in save::party(&globals_after, &entities_after)? {
        ensure!(c.experience == amount, "the edit did not survive the write");
    }
    for entry in save.package.entries() {
        if entry.name() != save::GLOBALS {
            ensure!(
                save.package.unpack(entry)? == written.read_file(entry.name())?,
                "{}: contents changed",
                entry.name()
            );
        }
    }
    std::fs::write(target, &bytes).with_context(|| target.display().to_string())?;
    writeln!(out, "wrote {}", target.display())?;
    Ok(())
}

fn check(args: &Args, out: &mut impl Write) -> Result<()> {
    ensure!(
        !args.positional.is_empty(),
        "usage: nautiloid-cli check <save>..."
    );
    let mut failures = 0;
    for path in &args.positional {
        match check_one(Path::new(path)) {
            Ok(report) => writeln!(out, "ok    {path}: {report}")?,
            Err(error) => {
                failures += 1;
                writeln!(out, "FAIL  {path}: {error:#}")?;
            }
        }
        out.flush()?;
    }
    ensure!(
        failures == 0,
        "{failures} of {} saves failed",
        args.positional.len()
    );
    Ok(())
}

fn check_one(path: &Path) -> Result<String> {
    let save = Save::open(path)?;
    let original = std::fs::read(&save.path)?;
    ensure!(
        save.package.write()? == original,
        "the rewritten package differs"
    );
    ensure!(
        save.package.compute_md5()? == save.package.stored_md5(),
        "the header checksum does not match the contents"
    );
    let (mut documents, mut nodes, mut attributes) = (0, 0, 0);
    for name in save.lsf_names() {
        let bytes = save.package.read_file(name)?;
        let sections = Sections::read(&bytes).with_context(|| name.to_owned())?;
        let lsf = Lsf::from_sections(sections.clone()).with_context(|| name.to_owned())?;
        ensure!(
            lsf.to_sections()? == sections,
            "{name}: rewritten sections differ"
        );
        for attr in 0..lsf.attr_count() as u32 {
            let decoded = lsf
                .value(attr)
                .with_context(|| format!("{name}: {}", lsf.attr_name(attr)))?;
            ensure!(
                value::encode(lsf.attr_type(attr), &decoded)? == lsf.attr_bytes(attr),
                "{name}: {} does not encode back",
                lsf.attr_name(attr)
            );
        }
        documents += 1;
        nodes += lsf.node_count();
        attributes += lsf.attr_count();
    }
    Ok(format!(
        "{documents} documents, {nodes} nodes, {attributes} values round-trip"
    ))
}

fn extract(args: &Args) -> Result<()> {
    args.expect(3, "extract <save> <file> <out>")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let bytes = save.package.read_file(&args.positional[1])?;
    let target = Path::new(&args.positional[2]);
    ensure!(!target.exists(), "{} already exists", target.display());
    std::fs::write(target, bytes).with_context(|| target.display().to_string())
}

fn rewrite(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(2, "rewrite <save> <out.lsv> [--reencode]")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let target = Path::new(&args.positional[1]);
    ensure!(!target.exists(), "{} already exists", target.display());
    let mut package = save.package.clone();
    if args.get("reencode").is_some() {
        if let Support::ReadOnly(why) = save.support() {
            bail!("{why}");
        }
        ensure!(
            package.entries().iter().all(|e| e.method() == Method::Zstd),
            "the save has files that are not stored with Zstandard"
        );
        for name in save.lsf_names() {
            let lsf = save.lsf(name)?;
            package.replace(name, &lsf.write()?)?;
        }
    }
    let bytes = package.write()?;
    // Read the result back before keeping it.
    let written = nautiloid_save_editor::lspk::Package::read(bytes.clone())?;
    ensure!(
        written.compute_md5()? == written.stored_md5(),
        "the written package fails its own checksum"
    );
    for entry in save.package.entries() {
        let name = entry.name();
        let (old, new) = (save.package.unpack(entry)?, written.read_file(name)?);
        let same = if name.ends_with(".lsf") {
            Sections::read(&old)? == Sections::read(&new)?
        } else {
            old == new
        };
        ensure!(same, "{name}: contents changed");
    }
    std::fs::write(target, &bytes).with_context(|| target.display().to_string())?;
    writeln!(
        out,
        "wrote {} ({} bytes; the original is {} bytes); contents verified equal",
        target.display(),
        bytes.len(),
        std::fs::metadata(&save.path)?.len()
    )?;
    Ok(())
}
