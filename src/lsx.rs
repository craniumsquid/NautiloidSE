//! `.lsx`: the XML form of the game's documents, which mods often carry beside or instead of
//! the binary `.lsf`. It is read into the same [`Lsf`] the rest of the editor reads, for the
//! attribute types an item catalog needs: text, translated text, identifiers, booleans and
//! integers. Others (vectors, matrices, floats) are left out.

use crate::lsf::build::{lsf, Node};
use crate::lsf::Lsf;
use crate::value::{DataType, Value};
use crate::xml::{self, Event};
use anyhow::{bail, Context, Result};

/// An attribute as an LSX file writes it, as a typed value, or `None` for a type not read.
fn value(
    kind: &str,
    value: Option<&str>,
    handle: Option<&str>,
    version: Option<&str>,
) -> Option<(DataType, Value)> {
    let text = || Value::Text(value.unwrap_or_default().to_owned());
    let int = || value.and_then(|v| v.trim().parse::<i64>().ok());
    Some(match kind {
        "FixedString" | "LSString" | "LSWString" | "guid" | "string" | "path" => {
            (DataType::FixedString, text())
        }
        "TranslatedString" => (
            DataType::TranslatedString,
            Value::Translated {
                version: version.and_then(|v| v.parse().ok()).unwrap_or(1),
                handle: handle.unwrap_or_default().to_owned(),
            },
        ),
        "bool" => (
            DataType::Bool,
            Value::Bool(value.is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1")),
        ),
        "int8" | "uint8" | "int16" | "uint16" | "int32" | "uint32" => (
            DataType::Int,
            Value::Int(int()?.clamp(i32::MIN as i64, i32::MAX as i64) as i32),
        ),
        "int64" | "uint64" => (DataType::Int64, Value::Int64(int()?)),
        _ => return None,
    })
}

/// Reads an LSX document. Each `region` becomes a root named after its `id`, holding its
/// node's attributes and children.
pub fn read(text: &str) -> Result<Lsf> {
    let events = xml::read(text)?;
    // The node being read, and those it is inside.
    let mut stack: Vec<Node> = Vec::new();
    let mut roots: Vec<Node> = Vec::new();
    let mut in_region = false;
    for event in &events {
        match event {
            Event::Start { name, .. } if name == "region" => in_region = true,
            Event::End { name } if name == "region" => in_region = false,
            Event::Start { name, .. } if name == "node" && in_region => {
                stack.push(Node::new(event.attribute("id").unwrap_or_default()));
            }
            Event::End { name } if name == "node" && in_region => {
                let Some(node) = stack.pop() else {
                    bail!("a node closes that never opened");
                };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => roots.push(node),
                }
            }
            Event::Start { name, .. } if name == "attribute" => {
                let Some(node) = stack.last_mut() else {
                    continue;
                };
                let id = event.attribute("id").unwrap_or_default();
                let kind = event.attribute("type").unwrap_or_default();
                if let Some((data_type, value)) = value(
                    kind,
                    event.attribute("value"),
                    event.attribute("handle"),
                    event.attribute("version"),
                ) {
                    *node = std::mem::take(node).attr(id, data_type, value);
                }
            }
            _ => {}
        }
    }
    let bytes = lsf(&roots).context("writing the document out")?;
    Lsf::parse(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_template() {
        let doc = read(
            r#"<?xml version="1.0" encoding="utf-8"?>
<save>
  <version major="4" minor="0" revision="9" build="331" lslib_meta="v1,bswap_guids" />
  <region id="Templates">
    <node id="Templates">
      <children>
        <node id="GameObjects">
          <attribute id="MapKey" type="FixedString" value="c304469f-6132-4304-a5a2-882a2820be3b" />
          <attribute id="Name" type="LSString" value="AMP_FeyCoin" />
          <attribute id="DisplayName" type="TranslatedString" handle="h56044379g6470g1baag292ege27c64f95732" version="1" />
          <attribute id="CanBePickedUp" type="bool" value="True" />
          <attribute id="maxStackAmount" type="int32" value="1" />
          <attribute id="Transform" type="fvec3" value="0 0 0" />
          <children>
            <node id="OnUsePeaceActions">
              <children>
                <node id="Action">
                  <attribute id="ActionType" type="int32" value="7" />
                </node>
              </children>
            </node>
          </children>
        </node>
      </children>
    </node>
  </region>
</save>"#,
        )
        .unwrap();
        let root = doc.root("Templates").unwrap();
        let template = doc.children_named(root, "GameObjects").next().unwrap();
        assert_eq!(
            doc.get(template, "Name").unwrap().as_str(),
            Some("AMP_FeyCoin")
        );
        assert_eq!(
            doc.get(template, "DisplayName").unwrap().as_str(),
            Some("h56044379g6470g1baag292ege27c64f95732")
        );
        assert_eq!(
            doc.get(template, "CanBePickedUp").unwrap().as_i64(),
            Some(1)
        );
        assert_eq!(
            doc.get(template, "maxStackAmount").unwrap().as_i64(),
            Some(1)
        );
        assert!(doc.get(template, "Transform").is_none(), "a type not read");
        let actions = doc
            .children_named(template, "OnUsePeaceActions")
            .next()
            .unwrap();
        assert_eq!(doc.children_named(actions, "Action").count(), 1);
    }
}
