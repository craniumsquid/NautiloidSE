//! What differs between two `.lsf` documents.
//!
//! Used to locate a field: take two saves a moment apart with one known change between them,
//! and see which attributes moved.
//!
//! Nodes have no stable identity across saves, so children are paired up in steps: identical
//! subtrees first (wherever they sit), then nodes with the same name and the same attributes,
//! then the same name and the same first attribute (usually an identifier), then by position.
//! What is left over on either side is reported as removed or added.

use crate::lsf::{AttrId, Lsf, NodeId};
use crate::value;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

/// One difference. Paths are as [`Lsf::path`] gives them, in the second document where the
/// node exists there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Value {
        path: String,
        attr: String,
        type_name: String,
        before: String,
        after: String,
    },
    AttrAdded {
        path: String,
        attr: String,
        type_name: String,
        value: String,
    },
    AttrRemoved {
        path: String,
        attr: String,
        type_name: String,
        value: String,
    },
    NodeAdded {
        path: String,
        /// Nodes in the added subtree, itself included.
        nodes: usize,
        /// The node's own attributes, to tell what it is.
        summary: String,
    },
    NodeRemoved {
        path: String,
        nodes: usize,
        summary: String,
    },
}

impl Change {
    pub fn path(&self) -> &str {
        match self {
            Change::Value { path, .. }
            | Change::AttrAdded { path, .. }
            | Change::AttrRemoved { path, .. }
            | Change::NodeAdded { path, .. }
            | Change::NodeRemoved { path, .. } => path,
        }
    }

    /// The path without sibling indices plus what changed there, for counting similar changes.
    pub fn kind(&self) -> String {
        let generic = strip_indices(self.path());
        match self {
            Change::Value { attr, .. } => format!("{generic} : {attr}"),
            Change::AttrAdded { attr, .. } => format!("{generic} : +{attr}"),
            Change::AttrRemoved { attr, .. } => format!("{generic} : -{attr}"),
            Change::NodeAdded { .. } => format!("+ {generic}"),
            Change::NodeRemoved { .. } => format!("- {generic}"),
        }
    }
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Change::Value {
                path,
                attr,
                type_name,
                before,
                after,
            } => write!(f, "~ {path} : {attr} ({type_name}) {before} -> {after}"),
            Change::AttrAdded {
                path,
                attr,
                type_name,
                value,
            } => write!(f, "+ {path} : {attr} ({type_name}) {value}"),
            Change::AttrRemoved {
                path,
                attr,
                type_name,
                value,
            } => write!(f, "- {path} : {attr} ({type_name}) {value}"),
            Change::NodeAdded {
                path,
                nodes,
                summary,
            } => write!(f, "+ {path} ({nodes} nodes) {summary}"),
            Change::NodeRemoved {
                path,
                nodes,
                summary,
            } => write!(f, "- {path} ({nodes} nodes) {summary}"),
        }
    }
}

/// `A/B[3]/C[0]` to `A/B/C`.
pub fn strip_indices(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut skipping = false;
    for c in path.chars() {
        match c {
            '[' => skipping = true,
            ']' => skipping = false,
            _ if !skipping => out.push(c),
            _ => {}
        }
    }
    out
}

/// The attribute's value as text: decoded where possible, hex otherwise.
pub fn show(lsf: &Lsf, attr: AttrId) -> String {
    match lsf.value(attr) {
        Ok(v) => v.to_string(),
        Err(_) => {
            let mut text = String::from("0x");
            let _ = value::write_hex(&mut text, lsf.attr_bytes(attr), 32);
            text
        }
    }
}

/// A node's own attributes on one line, shortened.
pub fn summarize(lsf: &Lsf, node: NodeId) -> String {
    let mut out = String::new();
    for &attr in lsf.attrs(node).iter().take(6) {
        if !out.is_empty() {
            out.push_str(", ");
        }
        let mut text = show(lsf, attr);
        if text.chars().count() > 48 {
            text = text.chars().take(47).collect::<String>() + "…";
        }
        out.push_str(&format!("{}={}", lsf.attr_name(attr), text));
    }
    if lsf.attrs(node).len() > 6 {
        out.push_str(", …");
    }
    out
}

/// Per-node hashes of one document.
struct Hashes {
    /// The node's name and own attributes.
    own: Vec<u64>,
    /// The node and everything under it.
    subtree: Vec<u64>,
    /// Nodes in the subtree, the node included.
    size: Vec<u32>,
}

fn hashes(lsf: &Lsf) -> Hashes {
    let n = lsf.node_count();
    let mut own = vec![0u64; n];
    let mut subtree = vec![0u64; n];
    let mut size = vec![1u32; n];
    // Children always have higher indices than their parent, so going backwards finishes
    // every child before its parent.
    for node in (0..n as NodeId).rev() {
        let mut hasher = DefaultHasher::new();
        lsf.node_name(node).hash(&mut hasher);
        for &attr in lsf.attrs(node) {
            lsf.attr_name(attr).hash(&mut hasher);
            lsf.attr_type(attr).hash(&mut hasher);
            lsf.attr_bytes(attr).hash(&mut hasher);
        }
        own[node as usize] = hasher.finish();
        for &child in lsf.children(node) {
            subtree[child as usize].hash(&mut hasher);
            size[node as usize] += size[child as usize];
        }
        subtree[node as usize] = hasher.finish();
    }
    Hashes { own, subtree, size }
}

struct Differ<'a> {
    a: &'a Lsf,
    b: &'a Lsf,
    ha: Hashes,
    hb: Hashes,
    changes: Vec<Change>,
}

impl Differ<'_> {
    fn node(&mut self, na: NodeId, nb: NodeId) {
        if self.ha.subtree[na as usize] == self.hb.subtree[nb as usize] {
            return;
        }
        if self.ha.own[na as usize] != self.hb.own[nb as usize] {
            self.attrs(na, nb);
        }
        // Copy the child lists out so the recursion can borrow `self` again.
        let ca = self.a.children(na).to_vec();
        let cb = self.b.children(nb).to_vec();
        self.children(&ca, &cb);
    }

    fn attrs(&mut self, na: NodeId, nb: NodeId) {
        let (a, b) = (self.a, self.b);
        let path = b.path(nb);
        // Pair attributes by name; a name that repeats pairs up in order.
        let mut theirs: HashMap<&str, VecDeque<AttrId>> = HashMap::new();
        for &attr in b.attrs(nb) {
            theirs.entry(b.attr_name(attr)).or_default().push_back(attr);
        }
        for &attr in a.attrs(na) {
            let name = a.attr_name(attr);
            match theirs.get_mut(name).and_then(VecDeque::pop_front) {
                Some(other) => {
                    if a.attr_type(attr) != b.attr_type(other)
                        || a.attr_bytes(attr) != b.attr_bytes(other)
                    {
                        self.changes.push(Change::Value {
                            path: path.clone(),
                            attr: name.to_owned(),
                            type_name: value::type_name(b.attr_type(other)),
                            before: show(a, attr),
                            after: show(b, other),
                        });
                    }
                }
                None => self.changes.push(Change::AttrRemoved {
                    path: path.clone(),
                    attr: name.to_owned(),
                    type_name: value::type_name(a.attr_type(attr)),
                    value: show(a, attr),
                }),
            }
        }
        for &attr in b.attrs(nb) {
            let name = b.attr_name(attr);
            if theirs
                .get_mut(name)
                .is_some_and(|queue| queue.front() == Some(&attr))
            {
                theirs.get_mut(name).map(VecDeque::pop_front);
                self.changes.push(Change::AttrAdded {
                    path: path.clone(),
                    attr: name.to_owned(),
                    type_name: value::type_name(b.attr_type(attr)),
                    value: show(b, attr),
                });
            }
        }
    }

    fn children(&mut self, ca: &[NodeId], cb: &[NodeId]) {
        let same = |this: &Self, x: NodeId, y: NodeId| {
            this.ha.subtree[x as usize] == this.hb.subtree[y as usize]
        };
        // Identical runs at the start and end.
        let mut start = 0;
        while start < ca.len() && start < cb.len() && same(self, ca[start], cb[start]) {
            start += 1;
        }
        let (mut end_a, mut end_b) = (ca.len(), cb.len());
        while end_a > start && end_b > start && same(self, ca[end_a - 1], cb[end_b - 1]) {
            end_a -= 1;
            end_b -= 1;
        }
        let (ca, cb) = (&ca[start..end_a], &cb[start..end_b]);
        if ca.is_empty() && cb.is_empty() {
            return;
        }

        // Identical subtrees that only moved.
        let mut by_hash: HashMap<u64, VecDeque<usize>> = HashMap::new();
        for (i, &node) in cb.iter().enumerate() {
            by_hash
                .entry(self.hb.subtree[node as usize])
                .or_default()
                .push_back(i);
        }
        let mut used = vec![false; cb.len()];
        let mut left_a = Vec::new();
        for &node in ca {
            match by_hash
                .get_mut(&self.ha.subtree[node as usize])
                .and_then(VecDeque::pop_front)
            {
                Some(i) => used[i] = true,
                None => left_a.push(node),
            }
        }
        let mut left_b: Vec<NodeId> = cb
            .iter()
            .zip(&used)
            .filter(|(_, &used)| !used)
            .map(|(&node, _)| node)
            .collect();

        // Pair what is left: first by name and own attributes, then by name and first
        // attribute, then by name and position.
        let mut pairs = Vec::new();
        for pass in 0..3 {
            if left_a.is_empty() || left_b.is_empty() {
                break;
            }
            let mut theirs: HashMap<(&str, Vec<u8>), VecDeque<usize>> = HashMap::new();
            for (i, &node) in left_b.iter().enumerate() {
                if let Some(key) = pair_key(self.b, &self.hb, node, pass) {
                    theirs.entry(key).or_default().push_back(i);
                }
            }
            let mut taken = vec![false; left_b.len()];
            left_a.retain(|&node| {
                let found = pair_key(self.a, &self.ha, node, pass)
                    .and_then(|key| theirs.get_mut(&key))
                    .and_then(VecDeque::pop_front);
                match found {
                    Some(i) => {
                        taken[i] = true;
                        pairs.push((node, left_b[i]));
                        false
                    }
                    None => true,
                }
            });
            let mut i = 0;
            left_b.retain(|_| {
                i += 1;
                !taken[i - 1]
            });
        }

        for node in left_a {
            self.changes.push(Change::NodeRemoved {
                path: self.a.path(node),
                nodes: self.ha.size[node as usize] as usize,
                summary: summarize(self.a, node),
            });
        }
        // Report paired nodes in the order they appear in the second document.
        pairs.sort_by_key(|&(_, b)| b);
        let mut added = left_b.into_iter().peekable();
        for (x, y) in pairs {
            while let Some(node) = added.next_if(|&node| node < y) {
                self.added(node);
            }
            self.node(x, y);
        }
        for node in added {
            self.added(node);
        }
    }

    fn added(&mut self, node: NodeId) {
        self.changes.push(Change::NodeAdded {
            path: self.b.path(node),
            nodes: self.hb.size[node as usize] as usize,
            summary: summarize(self.b, node),
        });
    }
}

/// What two nodes must share to be paired in the given pass.
fn pair_key<'a>(
    lsf: &'a Lsf,
    hashes: &Hashes,
    node: NodeId,
    pass: u8,
) -> Option<(&'a str, Vec<u8>)> {
    let name = lsf.node_name(node);
    match pass {
        0 => Some((name, hashes.own[node as usize].to_le_bytes().to_vec())),
        1 => {
            let &first = lsf.attrs(node).first()?;
            let mut key = lsf.attr_name(first).as_bytes().to_vec();
            key.push(0);
            key.extend_from_slice(lsf.attr_bytes(first));
            Some((name, key))
        }
        _ => Some((name, Vec::new())),
    }
}

/// Every difference between two documents, in the order of the second.
pub fn diff(a: &Lsf, b: &Lsf) -> Vec<Change> {
    let mut differ = Differ {
        a,
        b,
        ha: hashes(a),
        hb: hashes(b),
        changes: Vec::new(),
    };
    differ.children(a.roots(), b.roots());
    differ.changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsf::build::{self, Node};
    use crate::value::{DataType, Value};

    fn item(id: &str, amount: i32) -> Node {
        Node::new("Item")
            .attr("Template", DataType::FixedString, Value::Text(id.into()))
            .child(Node::new("Stack").attr("Amount", DataType::Int, Value::Int(amount)))
    }

    fn document(items: Vec<Node>, experience: i32) -> Lsf {
        let mut inventory = Node::new("Inventory");
        inventory.children = items;
        let character = Node::new("Character")
            .attr("Name", DataType::LSString, Value::Text("Tav".into()))
            .child(Node::new("Stats").attr("Experience", DataType::Int, Value::Int(experience)))
            .child(inventory);
        Lsf::parse(&build::lsf(&[Node::new("Party").child(character)]).unwrap()).unwrap()
    }

    #[test]
    fn identical_documents_have_no_changes() {
        let a = document(vec![item("Gold", 50), item("Potion", 2)], 100);
        assert!(diff(&a, &a.clone()).is_empty());
    }

    #[test]
    fn finds_a_changed_value_through_reordering() {
        let a = document(
            vec![item("Gold", 50), item("Potion", 2), item("Rope", 1)],
            100,
        );
        let b = document(
            vec![item("Rope", 1), item("Gold", 75), item("Potion", 2)],
            100,
        );
        let changes = diff(&a, &b);
        assert_eq!(changes.len(), 1, "{changes:#?}");
        assert_eq!(
            changes[0].to_string(),
            "~ Party/Character/Inventory/Item[1]/Stack : Amount (Int) 50 -> 75"
        );
        assert_eq!(
            changes[0].kind(),
            "Party/Character/Inventory/Item/Stack : Amount"
        );
    }

    #[test]
    fn reports_added_and_removed_nodes_and_attributes() {
        let a = document(vec![item("Gold", 50), item("Potion", 2)], 100);
        let mut b = document(vec![item("Gold", 50), item("Scroll", 1)], 250);
        let changes = diff(&a, &b);
        let text: Vec<String> = changes.iter().map(Change::to_string).collect();
        assert_eq!(
            text,
            [
                "~ Party/Character/Stats : Experience (Int) 100 -> 250",
                // Same name, different first attribute, so paired by position.
                "~ Party/Character/Inventory/Item[1] : Template (FixedString) \"Potion\" -> \"Scroll\"",
                "~ Party/Character/Inventory/Item[1]/Stack : Amount (Int) 2 -> 1",
            ]
        );

        let extra = document(
            vec![item("Gold", 50), item("Potion", 2), item("Rope", 3)],
            100,
        );
        let text: Vec<String> = diff(&a, &extra).iter().map(Change::to_string).collect();
        assert_eq!(
            text,
            ["+ Party/Character/Inventory/Item[2] (2 nodes) Template=\"Rope\""]
        );
        let text: Vec<String> = diff(&extra, &a).iter().map(Change::to_string).collect();
        assert_eq!(
            text,
            ["- Party/Character/Inventory/Item[2] (2 nodes) Template=\"Rope\""]
        );

        // An edit made through the document API shows up the same way.
        let stats = b.find(&["Party", "Character", "Stats"]).unwrap();
        let experience = b.attr(stats, "Experience").unwrap();
        b.set_value(experience, &Value::Int(100)).unwrap();
        assert_eq!(diff(&a, &b).len(), 2);
    }

    #[test]
    fn strips_indices() {
        assert_eq!(strip_indices("A/B[12]/C[0]"), "A/B/C");
        assert_eq!(strip_indices("A"), "A");
    }
}
