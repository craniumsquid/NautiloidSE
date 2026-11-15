//! Command-line tool for looking inside saves. It is for working out the format, not for
//! players: the editor itself has no need of it.

use anyhow::{anyhow, bail, ensure, Context, Result};
use nautiloid_save_editor::diff;
use nautiloid_save_editor::items;
use nautiloid_save_editor::lsf::{Lsf, NodeId, Sections};
use nautiloid_save_editor::lspk::Method;
use nautiloid_save_editor::newage::NewAge;
use nautiloid_save_editor::party::{self, Party};
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
      The party as the entity data has it: level, experience, ability scores, gold and
      Inspiration.
  items <save>
      What each character and the camp chest hold, with the amount of each stack and the
      camp supplies food is worth.
  edit <save> <folder> <name> [--xp WHO=N,...] [--level-up WHO,...]
       [--ability WHO:ABILITY=POINTS,...] [--gold WHO=N,...] [--inspiration N]
       [--approval WHO=N,...]
      Write a copy of the save as a new save called <name> under <folder>, with the party
      changed. WHO is a character's name or the start of their entity identifier.
      --xp sets total experience. --level-up entitles a character to one more level, which
      the game then offers. --ability sets the points put into an ability above 8.
      --gold sets the character's largest loose stack of gold.
  tables <save> <file.lsf>
      The tables in a document's entity data (its NewAge blob).
  rows <save> <file.lsf> <table> [--limit N]
      A table's rows as 32-bit numbers, with the entity each belongs to.
  entity <save> <file.lsf> <entity>
      Every row an entity owns, table by table.
  at <save> <file.lsf> <offset> [--len N]
      What a row's offset points at: the table and row there, and the bytes from it.
  set <save> <folder> <name> --u32 TABLE:ENTITY:OFFSET[@INDEX]=VALUE,... [--file FILE]
      Write a copy of the save as a new save called <name> under <folder>, with 32-bit
      fields of its entity data changed. ENTITY is the owner's identifier and OFFSET the
      field's position in the row, as the rows command shows them. With @INDEX, the row
      holds a pair of offsets at OFFSET and the change is to that array's INDEX-th value.
      #N in place of ENTITY picks the table's N-th row, for tables no entity owns.
  check <save>...
      Read each save, write it back in memory and compare.
  extract <save> <file> <out>
      Write one file's uncompressed contents to <out>.
  experiment <save> <folder> <name> <kind> [ITEM]
      Research: write a test save that adds to the entity data instead of overwriting it.
      Kinds: copy-item ITEM (a second copy of an unstacked item, in the same inventory);
      checks that change nothing a player would see: entity-list (the entity list moved),
      move-table [WHO] (the experience table moved, a party member's experience +100),
      loose-member ITEM, loose-slot-record ITEM, loose-reference ITEM (one of the item's
      inventory records, or its identifier, copied outside its table and pointed at),
      grown-tables (the inventory record tables grown, every pointer into them moved),
      old-list-row (the entity list moved, an unreferenced row of where it was rewritten).
  newage <save> <file.lsf> <out>
      Write a document's entity data (its NewAge blob) to <out>.
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
        "edit" => edit(
            &Args::parse(
                rest,
                &[],
                &[
                    "xp",
                    "level-up",
                    "ability",
                    "gold",
                    "inspiration",
                    "approval",
                ],
            )?,
            out,
        ),
        "items" => items(&Args::parse(rest, &[], &[])?, out),
        "tables" => tables(&Args::parse(rest, &[], &[])?, out),
        "rows" => rows(&Args::parse(rest, &[], &["limit"])?, out),
        "entity" => entity(&Args::parse(rest, &[], &[])?, out),
        "at" => at(&Args::parse(rest, &[], &["len"])?, out),
        "set" => set(&Args::parse(rest, &[], &["file", "u32"])?, out),
        "check" => check(&Args::parse(rest, &[], &[])?, out),
        "extract" => extract(&Args::parse(rest, &[], &[])?),
        "newage" => newage_blob(&Args::parse(rest, &[], &[])?),
        "experiment" => experiment(&Args::parse(rest, &[], &[])?, out),
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

fn print_party(party: &Party, out: &mut impl Write) -> Result<()> {
    for c in &party.characters {
        writeln!(
            out,
            "{}  {}",
            c.name.as_deref().unwrap_or("(no name)"),
            c.entity
        )?;
        let pending = if c.entitled_level > c.level {
            format!(" (entitled to level {})", c.entitled_level)
        } else {
            String::new()
        };
        let experience = match c.experience {
            Some(total) => format!("experience {total}"),
            None => "at camp".to_owned(),
        };
        writeln!(
            out,
            "    level {}{pending}, {experience}, template {}",
            c.level, c.template
        )?;
        let totals: Vec<String> = party::ABILITIES
            .iter()
            .zip(c.abilities)
            .map(|(name, total)| format!("{} {total}", &name[..3]))
            .collect();
        writeln!(out, "    abilities  {}", totals.join("  "))?;
        if let Some(allocated) = c.allocated {
            let points: Vec<String> = party::ABILITIES
                .iter()
                .zip(allocated)
                .map(|(name, points)| format!("{} {points}", &name[..3]))
                .collect();
            writeln!(
                out,
                "    allocated  {}  (points above {})",
                points.join("  "),
                party::ABILITY_BASE
            )?;
        }
        if !c.approval.is_empty() {
            let ratings: Vec<String> = c.approval.iter().map(|a| a.rating.to_string()).collect();
            writeln!(out, "    approval {}", ratings.join(", "))?;
        }
        let stacks: Vec<String> = c
            .gold
            .iter()
            .map(|g| {
                format!(
                    "{}{}",
                    g.amount,
                    if g.in_container {
                        " in a container"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        writeln!(
            out,
            "    gold {}{}",
            c.gold_total(),
            if stacks.len() > 1 {
                format!(" ({})", stacks.join(", "))
            } else {
                String::new()
            }
        )?;
    }
    writeln!(out, "party gold {}", party.gold_total())?;
    if let Some(inspiration) = party.inspiration {
        writeln!(
            out,
            "inspiration {} of {}",
            inspiration.amount, inspiration.max
        )?;
    }
    if let Some(supplies) = party.camp_supplies {
        writeln!(out, "camp supplies {supplies}")?;
    }
    Ok(())
}

fn party(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(1, "party <save>")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let globals = save.lsf(save::GLOBALS)?;
    let entities = NewAge::from_lsf(&globals)?;
    print_party(&party::read(&globals, &entities)?, out)
}

/// The party member called `who`: by name, or by the start of their entity identifier.
fn who<'a>(party: &'a Party, who: &str) -> Result<&'a party::Character> {
    let wanted = who.to_lowercase();
    let mut matches = party.characters.iter().filter(|c| {
        c.name
            .as_deref()
            .is_some_and(|n| n.to_lowercase() == wanted)
            || c.entity.to_string().starts_with(&wanted)
    });
    match (matches.next(), matches.next()) {
        (Some(character), None) => Ok(character),
        (None, _) => bail!("no party member called {who}"),
        (Some(_), Some(_)) => bail!("more than one party member matches {who}"),
    }
}

fn edit(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(
        3,
        "edit <save> <folder> <name> [--xp WHO=N,...] [--level-up WHO,...] \
         [--ability WHO:ABILITY=POINTS,...] [--gold WHO=N,...] [--inspiration N] \
         [--approval WHO=N,...]",
    )?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    if let Support::ReadOnly(why) = save.support() {
        bail!("{why}");
    }
    let mut globals = save.lsf(save::GLOBALS)?;
    let mut entities = NewAge::from_lsf(&globals)?;
    let before = party::read(&globals, &entities)?;
    let list = |name: &str| args.get(name).into_iter().flat_map(|l| l.split(','));
    let number = |text: &str| {
        text.parse::<u32>()
            .with_context(|| format!("{text} is not a number"))
    };

    for item in list("xp") {
        let (name, total) = item
            .split_once('=')
            .ok_or_else(|| anyhow!("--xp takes WHO=N"))?;
        party::set_experience(&mut entities, who(&before, name)?.entity, number(total)?)?;
    }
    for name in list("level-up") {
        let character = who(&before, name)?;
        party::set_entitled_level(&mut entities, character.entity, character.level + 1)?;
    }
    for item in list("ability") {
        let usage = || anyhow!("--ability takes WHO:ABILITY=POINTS, not {item}");
        let (target, points) = item.split_once('=').ok_or_else(usage)?;
        let (name, ability) = target.split_once(':').ok_or_else(usage)?;
        let ability = party::ABILITIES
            .iter()
            .position(|a| a.to_lowercase().starts_with(&ability.to_lowercase()))
            .ok_or_else(|| anyhow!("{ability} is not an ability"))?;
        party::set_allocated(
            &mut entities,
            who(&before, name)?.entity,
            ability,
            number(points)?,
        )?;
    }
    for item in list("gold") {
        let (name, amount) = item
            .split_once('=')
            .ok_or_else(|| anyhow!("--gold takes WHO=N"))?;
        let character = who(&before, name)?;
        // The reader lists a character's loose stacks first, largest first.
        let stack = character.gold.first().ok_or_else(|| {
            anyhow!("{name} carries no gold to change; adding items is not supported yet")
        })?;
        party::set_gold(&mut entities, stack.item, number(amount)?)?;
    }
    if let Some(amount) = args.get("inspiration") {
        party::set_inspiration(&mut entities, number(amount)?)?;
    }
    for item in list("approval") {
        let (name, rating) = item
            .split_once('=')
            .ok_or_else(|| anyhow!("--approval takes WHO=N"))?;
        let rating: i32 = rating
            .parse()
            .with_context(|| format!("{rating} is not a number"))?;
        let companion = who(&before, name)?;
        // The first rating is of the first player's character; a game for one has no other.
        let first = companion
            .approval
            .first()
            .ok_or_else(|| anyhow!("{name} has no approval rating to change"))?;
        party::set_approval(&mut entities, companion.entity, first.of, rating)?;
    }

    entities.store(&mut globals)?;
    let mut package = save.package.clone();
    package.replace(save::GLOBALS, &globals.write()?)?;
    let folder = save::export_copy(
        &save,
        &package,
        Path::new(&args.positional[1]),
        &args.positional[2],
    )?;
    // Show the party as the written save has it.
    let written = Save::open(&folder)?;
    let globals = written.lsf(save::GLOBALS)?;
    let after = party::read(&globals, &NewAge::from_lsf(&globals)?)?;
    writeln!(out, "wrote {}", folder.display())?;
    print_party(&after, out)
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
            "{row:>6}  @{:<9} {owner}  {}",
            entities.row_offset(id, row),
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

fn items(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(1, "items <save>")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let globals = save.lsf(save::GLOBALS)?;
    let party = party::read(&globals, &NewAge::from_lsf(&globals)?)?;
    let mut supplies = 0u64;
    for inventory in &party.inventories {
        let holder = match inventory.holder {
            party::Holder::Character(c) => party.characters[c].display_name(),
            party::Holder::CampChest => "Camp chest".to_owned(),
        };
        writeln!(out, "{holder}: {} items", inventory.items.len())?;
        for item in &inventory.items {
            let amount = match item.stacked {
                true => format!("x{}", item.amount),
                false => "one".to_owned(),
            };
            let supply = item
                .supply
                .map(|s| format!("  supplies {s} each"))
                .unwrap_or_default();
            let bagged = if item.in_container {
                "  (in a container)"
            } else {
                ""
            };
            writeln!(
                out,
                "  {amount:>6}  {}{supply}{bagged}  {}",
                item.stats, item.entity
            )?;
            supplies += item.supply.unwrap_or(0) as u64 * item.amount as u64;
        }
    }
    writeln!(out, "camp supplies by this count: {supplies}")?;
    if let Some(total) = party.camp_supplies {
        writeln!(out, "camp supplies as stored: {total}")?;
    }
    Ok(())
}

fn experiment(args: &Args, out: &mut impl Write) -> Result<()> {
    let usage = "experiment <save> <folder> <name> <kind> [ITEM]";
    ensure!(args.positional.len() >= 4, "usage: nautiloid-cli {usage}");
    let save = Save::open(Path::new(&args.positional[0]))?;
    if let Support::ReadOnly(why) = save.support() {
        bail!("{why}");
    }
    let mut globals = save.lsf(save::GLOBALS)?;
    let mut entities = NewAge::from_lsf(&globals)?;
    let item = || -> Result<value::Uuid> {
        let text = args
            .positional
            .get(4)
            .ok_or_else(|| anyhow!("this kind needs an item's entity"))?;
        value::Uuid::parse(text).ok_or_else(|| anyhow!("{text} is not an entity's identifier"))
    };
    match args.positional[3].as_str() {
        "copy-item" => {
            let copy = items::copy_item(&mut globals, &mut entities, item()?)?;
            writeln!(out, "copy {copy}")?;
        }
        "entity-list" => {
            entities.add_entities(&[])?;
        }
        "move-table" => {
            let table = entities
                .table(party::tables::EXPERIENCE)
                .ok_or_else(|| anyhow!("no experience table"))?;
            entities.move_table(table)?;
            let party = party::read(&globals, &entities)?;
            let first = &party.characters[0];
            let xp = first
                .experience
                .ok_or_else(|| anyhow!("not in the party"))?;
            party::set_experience(&mut entities, first.entity, xp + 100)?;
            writeln!(
                out,
                "{}: experience {xp} -> {}",
                first.display_name(),
                xp + 100
            )?;
        }
        "loose-member" => items::checks::loose_member_data(&mut entities, item()?)?,
        "loose-slot-record" => items::checks::loose_slot_data(&mut entities, item()?)?,
        "loose-reference" => items::checks::loose_reference(&mut entities, item()?)?,
        "grown-tables" => items::checks::grown_tables(&mut entities)?,
        "old-list-row" => items::checks::old_list_row(&mut entities)?,
        other => bail!("no experiment called {other}"),
    }

    entities.store(&mut globals)?;
    let mut package = save.package.clone();
    package.replace(save::GLOBALS, &globals.write()?)?;
    let folder = save::export_copy(
        &save,
        &package,
        Path::new(&args.positional[1]),
        &args.positional[2],
    )?;
    // Read back what was written.
    let written = Save::open(&folder)?;
    let globals = written.lsf(save::GLOBALS)?;
    let entities = NewAge::from_lsf(&globals)?;
    party::read(&globals, &entities)?;
    writeln!(out, "wrote {}", folder.display())?;
    Ok(())
}

fn newage_blob(args: &Args) -> Result<()> {
    args.expect(3, "newage <save> <file.lsf> <out>")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let lsf = save.lsf(&args.positional[1])?;
    let attr = NewAge::attribute(&lsf).ok_or_else(|| anyhow!("the document has no NewAge data"))?;
    std::fs::write(&args.positional[2], lsf.attr_bytes(attr))?;
    Ok(())
}

fn at(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(3, "at <save> <file.lsf> <offset> [--len N]")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let entities = NewAge::from_lsf(&save.lsf(&args.positional[1])?)?;
    let offset: u64 = args.positional[2]
        .parse()
        .map_err(|_| anyhow!("{} is not an offset", args.positional[2]))?;
    let len = args.number("len")?.unwrap_or(32);
    match entities.locate(offset) {
        Some((table, row, within)) => {
            let table = &entities.tables()[table];
            writeln!(
                out,
                "{} row {row} of {} ({} bytes each), {within} bytes in",
                table.name, table.rows, table.row_len
            )?;
            if let Some(entity) = entities.entity_at(offset) {
                writeln!(out, "entity {}", entities.entity_id(entity))?;
            }
        }
        None => writeln!(out, "in no table")?,
    }
    let bytes = entities.data(offset, len)?;
    writeln!(out, "{}", hex(bytes))?;
    writeln!(out, "{}", row_text(bytes))?;
    Ok(())
}

fn entity(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(3, "entity <save> <file.lsf> <entity>")?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    let entities = NewAge::from_lsf(&save.lsf(&args.positional[1])?)?;
    let id = value::Uuid::parse(&args.positional[2])
        .ok_or_else(|| anyhow!("{} is not an entity's identifier", args.positional[2]))?;
    let index = entities
        .entity_index(id)
        .ok_or_else(|| anyhow!("the document has no entity {id}"))?;
    for (table_id, table) in entities.tables().iter().enumerate() {
        if let Some(row) = entities.row_of(table_id, index) {
            writeln!(
                out,
                "{:<70} @{:<9} {}",
                table.name,
                entities.row_offset(table_id, row),
                row_text(entities.row(table_id, row))
            )?;
        }
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

/// One `TABLE:ENTITY:OFFSET=VALUE` from the set command.
/// Whose row a change is to.
enum Target {
    /// The row that belongs to this entity.
    Entity(value::Uuid),
    /// The row at this position in the table, written `#N`.
    Row(usize),
}

struct FieldChange {
    table: String,
    target: Target,
    offset: usize,
    /// With `OFFSET@INDEX`: which value of the array the row points at from `offset`.
    index: Option<usize>,
    value: u32,
}

impl FieldChange {
    fn entity(&self, entities: &NewAge, id: value::Uuid) -> Result<u32> {
        entities
            .entity_index(id)
            .ok_or_else(|| anyhow!("no entity {id} in the entity data"))
    }

    fn get(&self, entities: &NewAge) -> Result<u32> {
        match (&self.target, self.index) {
            (Target::Row(row), None) => entities.get_row_u32(&self.table, *row, self.offset),
            (Target::Entity(id), None) => {
                entities.get_u32(&self.table, self.entity(entities, *id)?, self.offset)
            }
            (Target::Entity(id), Some(index)) => entities
                .get_array_u32(&self.table, self.entity(entities, *id)?, self.offset)?
                .get(index)
                .copied()
                .ok_or_else(|| anyhow!("{}: no value {index} in the array", self.table)),
            (Target::Row(_), Some(_)) => bail!("@INDEX needs an entity, not a row number"),
        }
    }

    fn apply(&self, entities: &mut NewAge) -> Result<()> {
        match (&self.target, self.index) {
            (Target::Row(row), None) => {
                entities.set_row_u32(&self.table, *row, self.offset, self.value)
            }
            (Target::Entity(id), None) => {
                let entity = self.entity(entities, *id)?;
                entities.set_u32(&self.table, entity, self.offset, self.value)
            }
            (Target::Entity(id), Some(index)) => {
                let entity = self.entity(entities, *id)?;
                entities.set_array_u32(&self.table, entity, self.offset, index, self.value)
            }
            (Target::Row(_), Some(_)) => bail!("@INDEX needs an entity, not a row number"),
        }
    }

    fn place(&self) -> String {
        let whose = match &self.target {
            Target::Entity(id) => format!("of {id}"),
            Target::Row(row) => format!("row {row}"),
        };
        match self.index {
            None => format!("{whose} at {}", self.offset),
            Some(index) => format!("{whose} value {index} of the array at {}", self.offset),
        }
    }
}

fn parse_field_change(text: &str) -> Result<FieldChange> {
    let usage = || {
        anyhow!(
            "--u32 takes TABLE:ENTITY:OFFSET[@INDEX]=VALUE or TABLE:#ROW:OFFSET=VALUE, not {text}"
        )
    };
    let (target, value) = text.split_once('=').ok_or_else(usage)?;
    let mut parts = target.split(':');
    let (Some(table), Some(whose), Some(offset), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(usage());
    };
    let (offset, index) = match offset.split_once('@') {
        Some((offset, index)) => (offset, Some(index)),
        None => (offset, None),
    };
    let target = match whose.strip_prefix('#') {
        Some(row) => Target::Row(row.parse().with_context(|| format!("row {row}"))?),
        None => Target::Entity(
            value::Uuid::parse(whose)
                .ok_or_else(|| anyhow!("{whose} is not an entity identifier"))?,
        ),
    };
    Ok(FieldChange {
        table: table.to_owned(),
        target,
        offset: offset.parse().with_context(|| format!("offset {offset}"))?,
        index: index
            .map(|i| i.parse().with_context(|| format!("index {i}")))
            .transpose()?,
        value: value.parse().with_context(|| format!("value {value}"))?,
    })
}

fn set(args: &Args, out: &mut impl Write) -> Result<()> {
    args.expect(
        3,
        "set <save> <folder> <name> --u32 TABLE:ENTITY:OFFSET[@INDEX]=VALUE,... [--file FILE]",
    )?;
    let changes = args
        .get("u32")
        .ok_or_else(|| anyhow!("give the fields to change with --u32"))?
        .split(',')
        .map(parse_field_change)
        .collect::<Result<Vec<_>>>()?;
    let save = Save::open(Path::new(&args.positional[0]))?;
    if let Support::ReadOnly(why) = save.support() {
        bail!("{why}");
    }
    let file = args.get("file").unwrap_or(save::GLOBALS);
    let mut document = save.lsf(file)?;
    let mut entities = NewAge::from_lsf(&document)?;
    let mut report = Vec::new();
    for change in &changes {
        let before = change.get(&entities)?;
        change.apply(&mut entities)?;
        report.push(format!(
            "  {} {}: {before} -> {}",
            change.table,
            change.place(),
            change.value
        ));
    }
    entities.store(&mut document)?;
    let mut package = save.package.clone();
    package.replace(file, &document.write()?)?;
    let folder = save::export_copy(
        &save,
        &package,
        Path::new(&args.positional[1]),
        &args.positional[2],
    )?;

    // Read back what was written: the edits are there, the entity data still passes its
    // checksum (parsing checks it), and every other file is as it was.
    let written = Save::open(&folder)?;
    let after = NewAge::from_lsf(&written.lsf(file)?)?;
    for change in &changes {
        ensure!(
            change.get(&after)? == change.value,
            "{}: the edit did not survive the write",
            change.table
        );
    }
    for entry in save.package.entries() {
        if entry.name() != file {
            ensure!(
                save.package.unpack(entry)? == written.package.read_file(entry.name())?,
                "{}: contents changed",
                entry.name()
            );
        }
    }
    writeln!(out, "wrote {}", folder.display())?;
    for line in report {
        writeln!(out, "{line}")?;
    }
    writeln!(out, "  checksum updated; every other file is identical")?;
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
    let (mut documents, mut nodes, mut attributes, mut blobs) = (0, 0, 0, 0);
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
        // Parsing the entity data checks its checksum, which the game checks too.
        if let Some(attr) = NewAge::attribute(&lsf) {
            let entities = NewAge::parse(lsf.attr_bytes(attr).to_vec())
                .with_context(|| format!("{name}: entity data"))?;
            ensure!(
                entities.to_bytes() == lsf.attr_bytes(attr),
                "{name}: rewritten entity data differs"
            );
            blobs += 1;
        }
        documents += 1;
        nodes += lsf.node_count();
        attributes += lsf.attr_count();
    }
    Ok(format!(
        "{documents} documents, {nodes} nodes, {attributes} values round-trip; \
         {blobs} entity checksums hold"
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
