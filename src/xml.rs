//! A small XML reader for the files mods carry: `.lsx` documents, `.loca.xml` text and
//! `meta.lsx`. They are written by tools, so this reads elements, attributes, text, CDATA,
//! comments and the declaration, and nothing more (no DTDs, no namespaces).

use anyhow::{bail, Result};

/// One thing in a document, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// An element opens. An element written as `<x/>` is a `Start` then an `End`.
    Start {
        name: String,
        attributes: Vec<(String, String)>,
    },
    End {
        name: String,
    },
    /// Text between elements, with its entities replaced.
    Text(String),
}

impl Event {
    /// An attribute of a `Start`.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        match self {
            Event::Start { attributes, .. } => attributes
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str()),
            _ => None,
        }
    }
}

/// Replaces `&lt;` and the other entities, and character references.
pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        // Bytes, not characters: what follows may be any text.
        let Some(end) = rest.as_bytes()[..rest.len().min(12)]
            .iter()
            .position(|&b| b == b';')
        else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let replaced = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match replaced {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn attributes(text: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else {
            bail!("an attribute with no value in <{text}>");
        };
        let name = rest[..eq].trim().to_owned();
        let after = rest[eq + 1..].trim_start();
        let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'');
        let Some(quote) = quote else {
            bail!("an attribute value with no quotes in <{text}>");
        };
        let Some(close) = after[1..].find(quote) else {
            bail!("an attribute value with no end in <{text}>");
        };
        out.push((name, unescape(&after[1..1 + close])));
        rest = after[close + 2..].trim_start();
    }
    Ok(out)
}

/// Reads a whole document into its events.
pub fn read(text: &str) -> Result<Vec<Event>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            if !rest.trim().is_empty() {
                out.push(Event::Text(unescape(rest)));
            }
            break;
        };
        if open > 0 {
            let between = &rest[..open];
            if !between.trim().is_empty() {
                out.push(Event::Text(unescape(between)));
            }
        }
        rest = &rest[open..];
        if let Some(after) = rest.strip_prefix("<!--") {
            let Some(end) = after.find("-->") else {
                bail!("a comment with no end");
            };
            rest = &after[end + 3..];
        } else if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let Some(end) = after.find("]]>") else {
                bail!("a CDATA section with no end");
            };
            out.push(Event::Text(after[..end].to_owned()));
            rest = &after[end + 3..];
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            let Some(end) = rest.find('>') else {
                bail!("a declaration with no end");
            };
            rest = &rest[end + 1..];
        } else if let Some(after) = rest.strip_prefix("</") {
            let Some(end) = after.find('>') else {
                bail!("a closing tag with no end");
            };
            out.push(Event::End {
                name: after[..end].trim().to_owned(),
            });
            rest = &after[end + 1..];
        } else {
            // A tag's end is the first `>` outside quotes.
            let bytes = rest.as_bytes();
            let mut quote = None;
            let mut end = None;
            for (i, &b) in bytes.iter().enumerate().skip(1) {
                match (quote, b) {
                    (None, b'"' | b'\'') => quote = Some(b),
                    (Some(q), b) if b == q => quote = None,
                    (None, b'>') => {
                        end = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            let Some(end) = end else {
                bail!("a tag with no end");
            };
            let inside = &rest[1..end];
            let (inside, empty) = match inside.strip_suffix('/') {
                Some(inside) => (inside, true),
                None => (inside, false),
            };
            let split = inside
                .find(|c: char| c.is_whitespace())
                .unwrap_or(inside.len());
            let name = inside[..split].to_owned();
            out.push(Event::Start {
                name: name.clone(),
                attributes: attributes(&inside[split..])?,
            });
            if empty {
                out.push(Event::End { name });
            }
            rest = &rest[end + 1..];
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_elements_attributes_and_text() {
        let events = read(
            "\u{feff}<?xml version=\"1.0\"?>\n<!-- a note -->\n<list a=\"1 &amp; 2\" b='x>y'>\n\
             <item id=\"h1\" version=\"1\">Fey Coin&lt;br&gt;&#233;</item>\n\
             <empty/><raw><![CDATA[<b>bold</b>]]></raw></list>",
        )
        .unwrap();
        assert_eq!(
            events[0],
            Event::Start {
                name: "list".into(),
                attributes: vec![("a".into(), "1 & 2".into()), ("b".into(), "x>y".into())]
            }
        );
        assert_eq!(events[1].attribute("id"), Some("h1"));
        assert_eq!(events[2], Event::Text("Fey Coin<br>é".into()));
        assert_eq!(
            events[3],
            Event::End {
                name: "item".into()
            }
        );
        assert_eq!(
            events[4],
            Event::Start {
                name: "empty".into(),
                attributes: vec![]
            }
        );
        assert_eq!(
            events[5],
            Event::End {
                name: "empty".into()
            }
        );
        assert_eq!(events[7], Event::Text("<b>bold</b>".into()));
        assert_eq!(events.len(), 10);
        assert!(read("<a b=\"no end></a>").is_err());
        assert_eq!(unescape("a &unknown; b &"), "a &unknown; b &");
        assert_eq!(unescape("&──┈ x; &amp;"), "&──┈ x; &");
    }
}
