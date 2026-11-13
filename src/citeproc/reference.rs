//! Bibliographic items as the citation processor reads them: CSL-JSON, which is what
//! `bibtex::csl_json` writes and what Zotero, Pandoc and most tools exchange.

use std::collections::HashMap;

use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Name {
    pub family: String,
    pub given: String,
    pub dropping_particle: String,
    pub non_dropping_particle: String,
    pub suffix: String,
    /// A name that is one string (an institution): never inverted or initialised.
    pub literal: String,
    pub comma_suffix: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DateValue {
    /// One date, or two for a range. Each is `[year, month, day]` with 0 for "absent".
    pub parts: Vec<[i32; 3]>,
    pub literal: String,
    pub season: Option<i32>,
    pub circa: bool,
}

impl DateValue {
    pub fn year(&self) -> Option<i32> {
        self.parts.first().map(|p| p[0]).filter(|y| *y != 0)
    }
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty() && self.literal.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Text(String),
    Names(Vec<Name>),
    Date(DateValue),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reference {
    pub id: String,
    pub kind: String,
    pub fields: HashMap<String, Field>,
}

const NAME_VARS: &[&str] = &[
    "author", "editor", "translator", "container-author", "collection-editor", "composer", "director",
    "editorial-director", "illustrator", "interviewer", "original-author", "recipient", "reviewed-author",
    "chair", "compiler", "contributor", "curator", "executive-producer", "guest", "host", "narrator",
    "organizer", "performer", "producer", "script-writer", "series-creator",
];
const DATE_VARS: &[&str] = &["issued", "accessed", "event-date", "original-date", "submitted", "available-date"];

impl Reference {
    pub fn text(&self, var: &str) -> Option<&str> {
        match self.fields.get(var) {
            Some(Field::Text(t)) if !t.is_empty() => Some(t),
            _ => None,
        }
    }
    pub fn names(&self, var: &str) -> &[Name] {
        match self.fields.get(var) {
            Some(Field::Names(n)) => n,
            _ => &[],
        }
    }
    pub fn date(&self, var: &str) -> Option<&DateValue> {
        match self.fields.get(var) {
            Some(Field::Date(d)) if !d.is_empty() => Some(d),
            _ => None,
        }
    }

    /// Reads one CSL-JSON item.
    pub fn from_json(v: &Value) -> Option<Reference> {
        let obj = v.as_object()?;
        let id = match obj.get("id")? {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => return None,
        };
        let mut r = Reference { id, kind: obj.get("type").and_then(Value::as_str).unwrap_or("document").to_string(), fields: HashMap::new() };
        for (k, val) in obj {
            if k == "id" || k == "type" {
                continue;
            }
            let k = k.as_str();
            if NAME_VARS.contains(&k) {
                if let Some(arr) = val.as_array() {
                    let names: Vec<Name> = arr.iter().filter_map(name_from_json).collect();
                    if !names.is_empty() {
                        r.fields.insert(k.to_string(), Field::Names(names));
                    }
                }
            } else if DATE_VARS.contains(&k) {
                if let Some(d) = date_from_json(val) {
                    r.fields.insert(k.to_string(), Field::Date(d));
                }
            } else {
                let s = match val {
                    Value::String(s) => smart(s),
                    Value::Number(n) => n.to_string(),
                    _ => continue,
                };
                if !s.is_empty() {
                    r.fields.insert(k.to_string(), Field::Text(s));
                }
            }
        }
        Some(r)
    }
}

/// Straight apostrophes between letters become typographic ones, as Pandoc reads CSL-JSON text.
fn smart(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, ch) in c.iter().enumerate() {
        if *ch == '\'' && i > 0 && c[i - 1].is_alphanumeric() && c.get(i + 1).is_some_and(|n| n.is_alphanumeric()) {
            out.push('\u{2019}');
        } else {
            out.push(*ch);
        }
    }
    out
}

fn s(o: &serde_json::Map<String, Value>, k: &str) -> String {
    smart(o.get(k).and_then(Value::as_str).unwrap_or(""))
}

fn name_from_json(v: &Value) -> Option<Name> {
    let o = v.as_object()?;
    Some(Name {
        family: s(o, "family"),
        given: s(o, "given"),
        dropping_particle: s(o, "dropping-particle"),
        non_dropping_particle: s(o, "non-dropping-particle"),
        suffix: s(o, "suffix"),
        literal: s(o, "literal"),
        comma_suffix: o.get("comma-suffix").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn num(v: &Value) -> i32 {
    match v {
        Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
        Value::String(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn date_from_json(v: &Value) -> Option<DateValue> {
    let o = v.as_object()?;
    let mut d = DateValue::default();
    if let Some(parts) = o.get("date-parts").and_then(Value::as_array) {
        for p in parts {
            let a = p.as_array()?;
            let mut x = [0; 3];
            for (i, n) in a.iter().take(3).enumerate() {
                x[i] = num(n);
            }
            d.parts.push(x);
        }
    }
    d.literal = s(o, "literal");
    if d.literal.is_empty() {
        d.literal = s(o, "raw");
    }
    if let Some(sn) = o.get("season") {
        d.season = Some(num(sn));
    }
    d.circa = o.get("circa").is_some_and(|c| c.as_bool().unwrap_or(false) || c.as_str() == Some("true"));
    Some(d)
}

/// Reads a CSL-JSON array (or a single object).
pub fn references_from_json(text: &str) -> Result<Vec<Reference>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let items: Vec<&Value> = match &v {
        Value::Array(a) => a.iter().collect(),
        Value::Object(o) => match o.get("references").and_then(Value::as_array) {
            Some(a) => a.iter().collect(),
            None => vec![&v],
        },
        _ => return Err("not CSL-JSON".into()),
    };
    Ok(items.into_iter().filter_map(Reference::from_json).collect())
}
