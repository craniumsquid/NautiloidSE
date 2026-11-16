//! Reading the story state: `StorySave.bin`, the save of the game's story-script engine
//! (Osiris), and the flags in `Globals.lsf`. Read only: nothing here writes a save.
//!
//! The story file holds the compiled story (functions, rule nodes, adapters and goals) and
//! its databases: named tables of facts, such as the counters the story keeps. Its layout is
//! the one documented by the LSLib project's story reader; this reads the versions Patch 8
//! writes (1.14 and 1.15), with 16-bit type identifiers and a byte of flags before each
//! value.
//!
//! The rules run when the game changes a fact, not when a save is read, which is why this
//! only reads: a fact changed in a save would leave undone whatever the rules do about it.

use crate::lsf::Lsf;
use crate::save::{self, Save};
use crate::value::Uuid;
use anyhow::{anyhow, bail, ensure, Context, Result};
use std::collections::HashMap;

/// The newest version this reads: major 1, minor 15.
const VERSION: (u8, u8) = (1, 15);
/// The oldest version this reads: 1.14, which Patch 8 wrote before its second hotfix. The
/// two are laid out the same.
const OLDEST: (u8, u8) = (1, 14);
/// Strings are stored with every byte flipped by this.
const SCRAMBLE: u8 = 0xAD;

/// A value in a fact.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A column with nothing in it.
    None,
    Integer(i64),
    Real(f32),
    /// Text, or the name of something with its identifier on the end.
    Text(String),
}

impl Value {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(v) => Some(v),
            _ => None,
        }
    }
}

/// One of the story's tables of facts.
#[derive(Clone, Debug)]
pub struct Database {
    /// Its name, from the node that owns it: `DB_...` for the story's own, empty for the
    /// engine's working tables.
    pub name: String,
    /// How many columns its facts have.
    pub columns: usize,
    /// How many facts it holds.
    pub facts: usize,
    /// Where its facts start in the file.
    at: usize,
}

/// The story file, read as far as its databases' names and where their facts are. Facts are
/// read when asked for: a late-game save has over a million.
pub struct Story {
    bytes: Vec<u8>,
    /// What each type identifier is stored as: one of the five basic types.
    basic: HashMap<u16, u16>,
    /// Which type identifiers are enumerations, stored as their labels.
    enums: Vec<u16>,
    pub databases: Vec<Database>,
    by_name: HashMap<String, Vec<usize>>,
    /// The names the story gives identifiers, from every text that ends in one.
    pub names: HashMap<Uuid, String>,
}

const INTEGER: u16 = 1;
const INTEGER64: u16 = 2;
const REAL: u16 = 3;
const STRING: u16 = 4;
const GUID_STRING: u16 = 5;

/// A value's flag that says the value follows.
const IS_VALID: u8 = 0x08;

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    /// Names with an identifier on the end, as they are come across.
    names: HashMap<Uuid, String>,
}

/// How long an identifier is as text.
const ID_LEN: usize = 36;

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(len)
            .filter(|&end| end <= self.bytes.len())
            .ok_or_else(|| anyhow!("the story file ends early, at byte {}", self.at))?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn skip(&mut self, len: usize) -> Result<()> {
        self.take(len).map(|_| ())
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }

    fn count(&mut self) -> Result<usize> {
        Ok(self.u32()? as usize)
    }

    /// The bytes of a string, still scrambled, without its end.
    fn raw_string(&mut self, scramble: u8) -> Result<&'a [u8]> {
        let rest = &self.bytes[self.at.min(self.bytes.len())..];
        let len = rest
            .iter()
            .position(|&b| b == scramble)
            .ok_or_else(|| anyhow!("a string in the story file has no end"))?;
        self.at += len + 1;
        Ok(&rest[..len])
    }

    fn string(&mut self) -> Result<String> {
        Ok(plain(self.raw_string(SCRAMBLE)?))
    }

    fn skip_string(&mut self) -> Result<usize> {
        Ok(self.raw_string(SCRAMBLE)?.len())
    }

    /// Reads a value's text, or steps over it, noting the name if it ends in an identifier.
    fn text(&mut self, keep: bool) -> Result<Option<String>> {
        let raw = self.raw_string(SCRAMBLE)?;
        // Cheap to tell before unscrambling: an underscore, then eight digits and a hyphen.
        let named = raw.len() > ID_LEN + 1
            && raw[raw.len() - ID_LEN - 1] ^ SCRAMBLE == b'_'
            && raw[raw.len() - ID_LEN + 8] ^ SCRAMBLE == b'-';
        if !named && !keep {
            return Ok(None);
        }
        let text = plain(raw);
        if named {
            if let Some((name, id)) = split_name(&text) {
                self.names.entry(id).or_insert_with(|| name.to_owned());
            }
        }
        Ok(keep.then_some(text))
    }
}

fn plain(raw: &[u8]) -> String {
    let bytes: Vec<u8> = raw.iter().map(|b| b ^ SCRAMBLE).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// How values are stored, which depends on the types the file declares.
struct Types<'a> {
    basic: &'a HashMap<u16, u16>,
    enums: &'a [u16],
}

impl Types<'_> {
    /// Reads a value, or steps over it when `keep` is false.
    fn value(&self, r: &mut Reader, keep: bool) -> Result<Value> {
        let _index = r.u8()?;
        let flags = r.u8()?;
        if flags & IS_VALID == 0 {
            return Ok(Value::None);
        }
        match r.u8()? {
            // A reference to something by number.
            b'1' => {
                r.skip(2)?;
                let number = i32::from_le_bytes(r.take(4)?.try_into()?);
                Ok(Value::Integer(number as i64))
            }
            b'0' => {
                let id = r.u16()?;
                match self.basic.get(&id).copied().unwrap_or(id) {
                    0 => Ok(Value::None),
                    INTEGER => Ok(Value::Integer(
                        i32::from_le_bytes(r.take(4)?.try_into()?) as i64
                    )),
                    INTEGER64 => Ok(Value::Integer(i64::from_le_bytes(r.take(8)?.try_into()?))),
                    REAL => Ok(Value::Real(f32::from_le_bytes(r.take(4)?.try_into()?))),
                    STRING | GUID_STRING => {
                        if r.u8()? == 0 {
                            Ok(Value::None)
                        } else {
                            Ok(r.text(keep)?.map_or(Value::None, Value::Text))
                        }
                    }
                    other => bail!("a value of type {other}, which is not one this reads"),
                }
            }
            // An enumeration's label.
            b'e' => {
                let id = r.u16()?;
                ensure!(
                    self.enums.contains(&id),
                    "a label for type {id}, which is not an enumeration"
                );
                if keep {
                    Ok(Value::Text(r.string()?))
                } else {
                    r.skip_string()?;
                    Ok(Value::None)
                }
            }
            other => bail!("a value that starts with byte {other:#04x}"),
        }
    }

    fn values(&self, r: &mut Reader, keep: bool) -> Result<Vec<Value>> {
        let count = r.u8()? as usize;
        let mut out = Vec::with_capacity(if keep { count } else { 0 });
        for _ in 0..count {
            let value = self.value(r, keep)?;
            if keep {
                out.push(value);
            }
        }
        Ok(out)
    }

    /// Steps over a call: a name, its parameters, and the goal it belongs to.
    fn skip_call(&self, r: &mut Reader) -> Result<()> {
        if r.skip_string()? > 0 {
            if r.u8()? > 0 {
                self.values(r, false)?;
            }
            // Whether it is negated.
            r.skip(1)?;
        }
        r.skip(4)
    }

    fn skip_calls(&self, r: &mut Reader) -> Result<()> {
        for _ in 0..r.count()? {
            self.skip_call(r)?;
        }
        Ok(())
    }
}

/// A node, an entry point into it and a goal: three numbers.
const ENTRY: usize = 12;

impl Story {
    /// Reads a story file.
    pub fn parse(bytes: Vec<u8>) -> Result<Story> {
        let mut r = Reader {
            bytes: &bytes,
            at: 0,
            names: HashMap::new(),
        };
        // The header: a byte, a line of text that isn't scrambled, the version, and a
        // fixed space for the version as text.
        r.skip(1)?;
        r.raw_string(0)?;
        let version = (r.u8()?, r.u8()?);
        ensure!(
            (OLDEST..=VERSION).contains(&version),
            "the story file is version {}.{}; this reads {}.{} and {}.{}, which Patch 8 writes",
            version.0,
            version.1,
            OLDEST.0,
            OLDEST.1,
            VERSION.0,
            VERSION.1
        );
        ensure!(r.u8()? == 0, "the story file is big-endian");
        r.skip(1 + 0x80 + 4)?;

        // Types: a name, a number, and the type it is another name for.
        let mut alias: HashMap<u16, u16> = HashMap::new();
        for _ in 0..r.count()? {
            r.skip_string()?;
            let (index, of) = (r.u8()? as u16, r.u8()? as u16);
            alias.insert(index, of);
        }
        let mut basic: HashMap<u16, u16> = HashMap::new();
        for &index in alias.keys() {
            let mut at = index;
            for _ in 0..32 {
                match alias.get(&at) {
                    Some(&of) if of != 0 => at = of,
                    _ => break,
                }
            }
            basic.insert(index, at);
        }
        // Enumerations: the type, then each label and its number.
        let mut enums = Vec::new();
        for _ in 0..r.count()? {
            enums.push(r.u16()?);
            for _ in 0..r.count()? {
                r.skip_string()?;
                r.skip(8)?;
            }
        }
        let types = Types {
            basic: &basic,
            enums: &enums,
        };
        // Objects from an older game's editor: a name, a type and four numbers.
        for _ in 0..r.count()? {
            r.skip_string()?;
            r.skip(1 + 16)?;
        }
        // Functions: where each is used, then its name and parameter types.
        for _ in 0..r.count()? {
            r.skip(4 * 4 + 1 + 4 * 4)?;
            r.skip_string()?;
            let out = r.count()?;
            r.skip(out)?;
            let parameters = r.u8()? as usize;
            r.skip(parameters * 2)?;
        }
        // Nodes. A database's name is its node's.
        let mut names: HashMap<u32, (String, usize)> = HashMap::new();
        for _ in 0..r.count()? {
            let kind = r.u8()?;
            r.skip(4)?;
            let database = r.u32()?;
            let name = r.string()?;
            let columns = if name.is_empty() { 0 } else { r.u8()? as usize };
            if database != 0 && !name.is_empty() {
                names.entry(database).or_insert((name, columns));
            }
            match kind {
                // A database or a procedure: the nodes that use it.
                1 | 2 => {
                    let users = r.count()?;
                    r.skip(users * ENTRY)?;
                }
                // Queries have nothing more.
                3 | 8 | 9 => {}
                // A join of two nodes.
                4 | 5 => r.skip(ENTRY + 4 * 5 + ENTRY + 1 + 4 + ENTRY + 1)?,
                // A comparison, or a rule: both follow one node.
                6 | 7 => {
                    r.skip(ENTRY + 4 * 3 + ENTRY + 1)?;
                    if kind == 6 {
                        r.skip(2)?;
                        types.value(&mut r, false)?;
                        types.value(&mut r, false)?;
                        r.skip(4)?;
                    } else {
                        types.skip_calls(&mut r)?;
                        types.values(&mut r, false)?;
                        r.skip(4 + 1)?;
                    }
                }
                other => bail!("a story node of kind {other}"),
            }
        }
        // Adapters: constants, then two small maps.
        for _ in 0..r.count()? {
            r.skip(4)?;
            types.values(&mut r, false)?;
            let logical = r.u8()? as usize;
            r.skip(logical)?;
            let physical = r.u8()? as usize;
            r.skip(physical * 2)?;
        }
        // Databases: a number, the column types, and the facts.
        let mut databases = Vec::new();
        for _ in 0..r.count()? {
            let index = r.u32()?;
            let columns = r.u8()? as usize;
            r.skip(columns * 2)?;
            let facts = r.count()?;
            let at = r.at;
            for _ in 0..facts {
                types.values(&mut r, false)?;
            }
            let name = names
                .get(&index)
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            databases.push(Database {
                name,
                columns,
                facts,
                at,
            });
        }
        // Goals, then the calls that belong to none: read to check that the file is
        // understood to its end.
        for _ in 0..r.count()? {
            r.skip(4)?;
            r.skip_string()?;
            r.skip(1)?;
            let parents = r.count()?;
            r.skip(parents * 4)?;
            let subgoals = r.count()?;
            r.skip(subgoals * 4)?;
            r.skip(1)?;
            types.skip_calls(&mut r)?;
            types.skip_calls(&mut r)?;
        }
        types.skip_calls(&mut r)?;
        ensure!(
            r.at == bytes.len(),
            "{} bytes of the story file are left over",
            bytes.len() - r.at
        );

        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, database) in databases.iter().enumerate() {
            if !database.name.is_empty() {
                by_name.entry(database.name.clone()).or_default().push(i);
            }
        }
        let names = std::mem::take(&mut r.names);
        Ok(Story {
            bytes,
            basic,
            enums,
            databases,
            by_name,
            names,
        })
    }

    /// The facts of every database called `name` with `columns` columns.
    pub fn facts(&self, name: &str, columns: usize) -> Result<Vec<Vec<Value>>> {
        let types = Types {
            basic: &self.basic,
            enums: &self.enums,
        };
        let mut out = Vec::new();
        for &i in self.by_name.get(name).map(Vec::as_slice).unwrap_or(&[]) {
            let database = &self.databases[i];
            if database.columns != columns {
                continue;
            }
            let mut r = Reader {
                bytes: &self.bytes,
                at: database.at,
                names: HashMap::new(),
            };
            for _ in 0..database.facts {
                out.push(types.values(&mut r, true).context(name.to_owned())?);
            }
        }
        Ok(out)
    }
}

/// The identifier on the end of a name such as `ORI_State_Something_<identifier>`, and the
/// name before it.
pub fn split_name(text: &str) -> Option<(&str, Uuid)> {
    let at = text.len().checked_sub(36)?;
    if !text.is_char_boundary(at) {
        return None;
    }
    let id = Uuid::parse(&text[at..])?;
    Some((text[..at].strip_suffix('_')?, id))
}

/// The flags the save keeps, by identifier: whether each is set. A flag the game has never
/// touched is not there.
pub fn flags(globals: &Lsf) -> Vec<(Uuid, bool)> {
    let Some(map) = globals.find(&["OsirisVariableHelper", "VariableManager", "FlagMap"]) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for container in globals.children_named(map, "FlagContainerMap") {
        for object in globals.children_named(container, "Object") {
            let Some(id) = globals.get(object, "MapKey").and_then(|v| v.as_uuid()) else {
                continue;
            };
            let set = globals
                .children_named(object, "FlagContainerMap")
                .filter_map(|n| globals.get(n, "Flags"))
                .any(|v| v.as_i64() == Some(1));
            out.push((id, set));
        }
    }
    out
}

/// A flag the save keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct Flag {
    pub id: Uuid,
    /// What the story calls it, if the story names it anywhere.
    pub name: Option<String>,
    pub set: bool,
}

/// One of the things a tally counts, and whether it has happened.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// The flag's name, such as `ORI_Shadowheart_State_NightsongPoint_GaveNightOrchid`.
    pub name: String,
    pub done: bool,
}

/// A count the story keeps of things that happened, with the things themselves: the points
/// toward Shadowheart sparing the Nightsong, for one.
#[derive(Clone, Debug, PartialEq)]
pub struct Tally {
    /// What the story calls it, such as `ORI_Shadowheart_NightsongPoint`.
    pub name: String,
    /// The counter that goes with it, and where it stands.
    pub counter: Option<(String, i64)>,
    pub events: Vec<Event>,
    /// Whether the story has marked the count as enough, where it has a flag for that.
    pub enough: Option<bool>,
}

/// The story state the editor shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// The story's counters by name, in the story's order.
    pub counters: Vec<(String, i64)>,
    pub tallies: Vec<Tally>,
    /// The flags the save keeps, named ones first by name.
    pub flags: Vec<Flag>,
}

/// The database of counters: a name and a number.
const COUNTERS: &str = "DB_GlobalCounter";
/// What the databases that list a tally's flags are called: `DB_<tally>_ConditionFlags`.
const TALLY_FLAGS: &str = "_ConditionFlags";
/// The end of the flag that says a tally has reached what it needs.
const ENOUGH: &str = "HasEnoughPoints";

impl State {
    /// Reads the story state of a save: its story file and the flags in `globals`.
    pub fn read(save: &Save, globals: &Lsf) -> Result<State> {
        let story = Story::parse(save.package.read_file(save::STORY)?).context(save::STORY)?;
        State::from_parts(&story, &flags(globals))
    }

    fn from_parts(story: &Story, kept: &[(Uuid, bool)]) -> Result<State> {
        let set: HashMap<Uuid, bool> = kept.iter().copied().collect();
        let counters: Vec<(String, i64)> = story
            .facts(COUNTERS, 2)?
            .into_iter()
            .filter_map(|fact| Some((fact[0].as_str()?.to_owned(), fact[1].as_i64()?)))
            .collect();

        let mut tallies = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for database in &story.databases {
            let Some(name) = database
                .name
                .strip_prefix("DB_")
                .and_then(|n| n.strip_suffix(TALLY_FLAGS))
            else {
                continue;
            };
            if database.columns != 1 || !seen.insert(name) {
                continue;
            }
            let named: Vec<(String, Uuid)> = story
                .facts(&database.name, 1)?
                .iter()
                .filter_map(|fact| split_name(fact[0].as_str()?))
                .map(|(name, id)| (name.to_owned(), id))
                .collect();
            if named.is_empty() {
                continue;
            }
            // The flag for having enough is named like the others, up to its last part.
            let prefix = named[0].0.rsplit_once('_').map_or("", |(before, _)| before);
            let enough_name = format!("{prefix}_{ENOUGH}");
            let enough = story
                .names
                .iter()
                .find(|(_, name)| **name == enough_name)
                .map(|(id, _)| set.get(id).copied().unwrap_or(false));
            // The counter is called as the tally is, give or take an ending: the Nightsong
            // tally is "NightsongPoint" and its counter "NightsongPoints".
            let counter = counters
                .iter()
                .filter(|(counter, _)| counter.starts_with(name))
                .min_by_key(|(counter, _)| counter.len())
                .cloned();
            tallies.push(Tally {
                name: name.to_owned(),
                counter,
                events: named
                    .into_iter()
                    .map(|(name, id)| Event {
                        name,
                        done: set.get(&id).copied().unwrap_or(false),
                    })
                    .collect(),
                enough,
            });
        }
        tallies.sort_by(|a, b| a.name.cmp(&b.name));

        let mut flags: Vec<Flag> = kept
            .iter()
            .map(|&(id, set)| Flag {
                id,
                name: story.names.get(&id).cloned(),
                set,
            })
            .collect();
        flags.sort_by(|a, b| {
            (a.name.is_none(), &a.name, a.id.to_string()).cmp(&(
                b.name.is_none(),
                &b.name,
                b.id.to_string(),
            ))
        });
        Ok(State {
            counters,
            tallies,
            flags,
        })
    }
}

/// Builds a small story file and flag map from scratch, for tests.
pub mod build {
    use super::*;
    use crate::lsf::build::Node;
    use crate::value::{DataType, Value as Field};

    fn text(out: &mut Vec<u8>, text: &str) {
        out.extend(text.bytes().map(|b| b ^ SCRAMBLE));
        out.push(SCRAMBLE);
    }

    fn value(out: &mut Vec<u8>, value: &Value) {
        // No place in a rule, and valid.
        out.extend([0xFF, IS_VALID | 1]);
        match value {
            Value::None => {
                out.pop();
                out.push(1);
            }
            Value::Integer(v) => {
                out.push(b'0');
                out.extend(INTEGER.to_le_bytes());
                out.extend((*v as i32).to_le_bytes());
            }
            Value::Real(v) => {
                out.push(b'0');
                out.extend(REAL.to_le_bytes());
                out.extend(v.to_le_bytes());
            }
            Value::Text(v) => {
                out.push(b'0');
                // The second kind of text is a type of its own, another name for the first.
                let kind = if split_name(v).is_some() {
                    6u16
                } else {
                    STRING
                };
                out.extend(kind.to_le_bytes());
                out.push(1);
                text(out, v);
            }
        }
    }

    /// A story file holding these databases: each a name and its facts, which all have as
    /// many columns as the first.
    pub fn file(databases: &[(&str, Vec<Vec<Value>>)]) -> Vec<u8> {
        let mut out = vec![0u8];
        out.extend(b"Osiris save file dd. 01/01/26 00:00:00. Version 1.8.\0");
        out.extend([VERSION.0, VERSION.1, 0, 0]);
        let mut version = [0u8; 0x80];
        version[..4].copy_from_slice(b"1.15");
        out.extend(version);
        out.extend(0u32.to_le_bytes());
        // Types: the second kind of text, and another name for it.
        out.extend(2u32.to_le_bytes());
        for (name, index, alias) in [("GUIDSTRING", 5u8, 0u8), ("FLAG", 6, 5)] {
            text(&mut out, name);
            out.extend([index, alias]);
        }
        // No enumerations, old objects or functions.
        for _ in 0..3 {
            out.extend(0u32.to_le_bytes());
        }
        // A node for each database, which gives it its name.
        out.extend((databases.len() as u32).to_le_bytes());
        for (i, (name, facts)) in databases.iter().enumerate() {
            out.push(1);
            out.extend((i as u32 + 1).to_le_bytes());
            out.extend((i as u32 + 1).to_le_bytes());
            text(&mut out, name);
            out.push(facts.first().map_or(1, Vec::len) as u8);
            out.extend(0u32.to_le_bytes());
        }
        // No adapters.
        out.extend(0u32.to_le_bytes());
        out.extend((databases.len() as u32).to_le_bytes());
        for (i, (_, facts)) in databases.iter().enumerate() {
            out.extend((i as u32 + 1).to_le_bytes());
            let columns = facts.first().map_or(1, Vec::len);
            out.push(columns as u8);
            for _ in 0..columns {
                out.extend(STRING.to_le_bytes());
            }
            out.extend((facts.len() as u32).to_le_bytes());
            for fact in facts {
                out.push(fact.len() as u8);
                for column in fact {
                    value(&mut out, column);
                }
            }
        }
        // No goals, and no calls outside them.
        out.extend(0u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out
    }

    /// The identifier of the sample's flag number `n`.
    pub fn flag_id(n: u8) -> Uuid {
        Uuid::parse(&format!("f1a90000-0000-4000-8000-0000000000{n:02x}")).expect("an identifier")
    }

    fn flag_name(name: &str, n: u8) -> Value {
        Value::Text(format!("{name}_{}", flag_id(n)))
    }

    /// The sample story: two counters, and a tally of three things, two of them done.
    pub fn sample() -> Vec<u8> {
        let count = |name: &str, n: i64| vec![Value::Text(name.to_owned()), Value::Integer(n)];
        file(&[
            (
                COUNTERS,
                vec![
                    count("ORI_Shadowheart_NightsongPoints", 2),
                    count("Camp_Rest_Count", 6),
                ],
            ),
            (
                "DB_ORI_Shadowheart_NightsongPoint_ConditionFlags",
                vec![
                    vec![flag_name(
                        "ORI_Shadowheart_State_NightsongPoint_GaveNightOrchid",
                        1,
                    )],
                    vec![flag_name(
                        "ORI_Shadowheart_State_NightsongPoint_AteNoblestalk",
                        2,
                    )],
                    vec![flag_name(
                        "ORI_Shadowheart_State_NightsongPoint_AgreedToTrials",
                        3,
                    )],
                ],
            ),
            (
                "DB_Other",
                vec![
                    vec![flag_name(
                        "ORI_Shadowheart_State_NightsongPoint_HasEnoughPoints",
                        4,
                    )],
                    vec![flag_name("GLO_Origin_PartOfTheTeam_Shadowheart", 5)],
                ],
            ),
        ])
    }

    /// The sample's flags, as `Globals.lsf` keeps them: flags 1, 2 and 5 set, 4 cleared, 3
    /// never touched, and one the story has no name for.
    pub fn flag_map() -> Node {
        let mut container = Node::new("FlagContainerMap");
        for (n, set) in [(1, true), (2, true), (4, false), (5, true), (9, true)] {
            container = container.child(
                Node::new("Object")
                    .attr("MapKey", DataType::Uuid, Field::Uuid(flag_id(n)))
                    .child(Node::new("FlagContainerMap").attr(
                        "Flags",
                        DataType::Bool,
                        Field::Bool(set),
                    )),
            );
        }
        Node::new("OsirisVariableHelper")
            .child(Node::new("VariableManager").child(Node::new("FlagMap").child(container)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_databases_and_their_facts() {
        let story = Story::parse(build::sample()).unwrap();
        let names: Vec<&str> = story.databases.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "DB_GlobalCounter",
                "DB_ORI_Shadowheart_NightsongPoint_ConditionFlags",
                "DB_Other"
            ]
        );
        assert_eq!(
            (story.databases[0].columns, story.databases[0].facts),
            (2, 2)
        );
        let counters = story.facts("DB_GlobalCounter", 2).unwrap();
        assert_eq!(
            counters[0],
            [
                Value::Text("ORI_Shadowheart_NightsongPoints".to_owned()),
                Value::Integer(2)
            ]
        );
        assert!(story.facts("DB_GlobalCounter", 3).unwrap().is_empty());
        assert!(story.facts("DB_Missing", 1).unwrap().is_empty());
        // Every text that ends in an identifier names it.
        assert_eq!(story.names.len(), 5);
        assert_eq!(
            story.names[&build::flag_id(5)],
            "GLO_Origin_PartOfTheTeam_Shadowheart"
        );
    }

    #[test]
    fn refuses_what_it_does_not_understand() {
        let mut older = build::sample();
        let at = older.iter().position(|&b| b == VERSION.1).unwrap();
        older[at] = 13;
        let why = Story::parse(older).err().unwrap().to_string();
        assert!(why.contains("version 1.13"), "{why}");

        let mut longer = build::sample();
        longer.push(0);
        let why = Story::parse(longer).err().unwrap().to_string();
        assert!(why.contains("left over"), "{why}");

        let mut shorter = build::sample();
        shorter.truncate(shorter.len() - 3);
        assert!(Story::parse(shorter).is_err());
    }

    #[test]
    fn puts_counters_tallies_and_flags_together() {
        let story = Story::parse(build::sample()).unwrap();
        let globals =
            crate::lsf::Lsf::parse(&crate::lsf::build::lsf(&[build::flag_map()]).unwrap()).unwrap();
        let kept = flags(&globals);
        assert_eq!(kept.len(), 5);
        let state = State::from_parts(&story, &kept).unwrap();
        assert_eq!(
            state.counters,
            [
                ("ORI_Shadowheart_NightsongPoints".to_owned(), 2),
                ("Camp_Rest_Count".to_owned(), 6)
            ]
        );
        let [tally] = &state.tallies[..] else {
            panic!("one tally, not {}", state.tallies.len());
        };
        assert_eq!(tally.name, "ORI_Shadowheart_NightsongPoint");
        assert_eq!(
            tally.counter,
            Some(("ORI_Shadowheart_NightsongPoints".to_owned(), 2))
        );
        let done: Vec<bool> = tally.events.iter().map(|e| e.done).collect();
        assert_eq!(done, [true, true, false], "the third was never touched");
        assert_eq!(
            tally.events[0].name,
            "ORI_Shadowheart_State_NightsongPoint_GaveNightOrchid"
        );
        assert_eq!(tally.enough, Some(false), "kept, but cleared");
        // Named flags by name, then the one the story has no name for.
        let names: Vec<Option<&str>> = state.flags.iter().map(|f| f.name.as_deref()).collect();
        assert_eq!(names[0], Some("GLO_Origin_PartOfTheTeam_Shadowheart"));
        assert_eq!(names[4], None);
        assert!(state.flags[4].set);
    }

    #[test]
    fn splits_a_name_from_its_identifier() {
        let (name, id) = split_name("S_Player_f1a90000-0000-4000-8000-000000000001").unwrap();
        assert_eq!((name, id), ("S_Player", build::flag_id(1)));
        assert_eq!(split_name("ORI_Shadowheart_NightsongPoints"), None);
        assert_eq!(split_name("short"), None);
    }
}
