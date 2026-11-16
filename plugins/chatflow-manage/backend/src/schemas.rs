//! The shape of a stage's answer: read from its JSON Schema (`data/schemas/<stage>.json`), sent to
//! the model as a grammar, and checked on the way back.
//!
//! Property order is the order the model writes, and it matters: the evidence is written before
//! the decision it supports (`because` before `relation`). JSON objects lose their order on the way
//! through the frame (parameters are relayed as parsed JSON, whose maps come out sorted), so the
//! shape goes as a GBNF grammar (a string, relayed as it is), generated from the schema. The order
//! is the schema's `required` list, which therefore names every property. Same mechanism as
//! CortexCode's chat flow; this one also knows booleans and whole numbers.

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Str,
    /// A string of at most this many characters: free text the model must close.
    StrMax(usize),
    OneOf(Vec<String>),
    Bool,
    /// A whole number, zero or more.
    Int,
    List(Box<Shape>),
    /// A list of at most this many items (0: always empty).
    ListMax(Box<Shape>, usize),
    /// Properties in the order they are written; all required.
    Obj(Vec<(String, Shape)>),
}

/// Placeholder of an enum filled at run time with a plan's task numbers (`depends_on`).
pub const TASKS: &str = "@tasks";
/// Placeholder of an enum of the point numbers CONVERGE was given.
pub const POINTS: &str = "@points";
/// Placeholder of an enum of the files a diff changes: an answer can only name a file that is there.
pub const FILES: &str = "@files";

impl Shape {
    /// A top-level property of an object shape.
    pub fn field(&self, name: &str) -> Option<&Shape> {
        match self {
            Shape::Obj(properties) => properties.iter().find(|(key, _)| key == name).map(|(_, shape)| shape),
            _ => None,
        }
    }

    /// The shape with every enum holding only `placeholder` filled with `values`. With nothing to
    /// choose from, a list that would hold such a value can only be empty.
    pub fn filled(&self, placeholder: &str, values: &[String]) -> Shape {
        fn fill(shape: &Shape, placeholder: &str, values: &[String]) -> Option<Shape> {
            match shape {
                Shape::OneOf(current) if current.len() == 1 && current[0] == placeholder => (!values.is_empty()).then(|| Shape::OneOf(values.to_vec())),
                Shape::Obj(properties) => {
                    let mut kept = Vec::new();
                    for (key, value) in properties {
                        // An object cannot be written without one of its properties.
                        kept.push((key.clone(), fill(value, placeholder, values)?));
                    }
                    Some(Shape::Obj(kept))
                }
                Shape::List(item) => Some(match fill(item, placeholder, values) {
                    Some(item) => Shape::List(Box::new(item)),
                    None => Shape::ListMax(Box::new(Shape::Str), 0),
                }),
                Shape::ListMax(item, most) => Some(match fill(item, placeholder, values) {
                    Some(item) => Shape::ListMax(Box::new(item), *most),
                    None => Shape::ListMax(Box::new(Shape::Str), 0),
                }),
                other => Some(other.clone()),
            }
        }
        fill(self, placeholder, values).unwrap_or_else(|| self.clone())
    }

    /// The same shape with a top-level list capped at `most` items.
    pub fn capped(&self, field: &str, most: usize) -> Shape {
        match self {
            Shape::Obj(properties) => Shape::Obj(
                properties
                    .iter()
                    .map(|(key, shape)| match shape {
                        Shape::List(item) if key == field => (key.clone(), Shape::ListMax(item.clone(), most)),
                        Shape::ListMax(item, current) if key == field => (key.clone(), Shape::ListMax(item.clone(), most.min(*current))),
                        other => (key.clone(), other.clone()),
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// An answer of this shape with every list holding one item: what the probe turns into the
    /// contract's type, to show the schema carries the fields the contract needs.
    pub fn sample(&self) -> Value {
        match self {
            Shape::Str | Shape::StrMax(_) => json!("sample"),
            Shape::OneOf(values) => json!(values.first().cloned().unwrap_or_default()),
            Shape::Bool => json!(true),
            Shape::Int => json!(1),
            Shape::List(item) => json!([item.sample()]),
            Shape::ListMax(_, 0) => json!([]),
            Shape::ListMax(item, _) => json!([item.sample()]),
            Shape::Obj(properties) => {
                let mut map = Map::new();
                for (name, shape) in properties {
                    map.insert(name.clone(), shape.sample());
                }
                Value::Object(map)
            }
        }
    }

    /// Whether `value` has this shape; the first difference otherwise. The grammar already holds
    /// the model to it, but a model on another computer may run an engine that does not.
    pub fn check(&self, value: &Value) -> Result<(), String> {
        fn at(path: &str) -> String {
            if path.is_empty() { "the answer".to_owned() } else { format!("`{path}`") }
        }
        fn walk(shape: &Shape, value: &Value, path: &str) -> Result<(), String> {
            match shape {
                Shape::Str | Shape::StrMax(_) => value.is_string().then_some(()).ok_or_else(|| format!("{} must be text", at(path))),
                Shape::OneOf(values) => match value.as_str() {
                    Some(text) if values.iter().any(|known| known == text) => Ok(()),
                    _ => Err(format!("{} must be one of: {}", at(path), values.join(", "))),
                },
                Shape::Bool => value.is_boolean().then_some(()).ok_or_else(|| format!("{} must be true or false", at(path))),
                Shape::Int => value.is_u64().then_some(()).ok_or_else(|| format!("{} must be a whole number", at(path))),
                Shape::List(item) | Shape::ListMax(item, _) => {
                    let items = value.as_array().ok_or_else(|| format!("{} must be a list", at(path)))?;
                    if let Shape::ListMax(_, most) = shape
                        && items.len() > *most
                    {
                        return Err(format!("{} holds more than {most} items", at(path)));
                    }
                    for (index, entry) in items.iter().enumerate() {
                        walk(item, entry, &format!("{path}[{index}]"))?;
                    }
                    Ok(())
                }
                Shape::Obj(properties) => {
                    let map = value.as_object().ok_or_else(|| format!("{} must be an object", at(path)))?;
                    for (name, shape) in properties {
                        let inner = if path.is_empty() { name.clone() } else { format!("{path}.{name}") };
                        let entry = map.get(name).ok_or_else(|| format!("`{inner}` is missing"))?;
                        walk(shape, entry, &inner)?;
                    }
                    Ok(())
                }
            }
        }
        walk(self, value, "")
    }
}

/// Read a JSON Schema (the subset the stages use: object, string, boolean, integer, enum of
/// strings, array). Every property of an object must be in `required`, which gives the order.
pub fn shape_from_schema(schema: &Value) -> Result<Shape, String> {
    if let Some(values) = schema.get("enum") {
        let values: Vec<String> = values
            .as_array()
            .ok_or("`enum` must be an array")?
            .iter()
            .map(|value| value.as_str().map(str::to_owned).ok_or("enum values must be strings"))
            .collect::<Result<_, _>>()?;
        if values.is_empty() {
            return Err("empty `enum`".into());
        }
        return Ok(Shape::OneOf(values));
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("string") => Ok(match schema.get("maxLength").and_then(Value::as_u64) {
            Some(max) => Shape::StrMax(max as usize),
            None => Shape::Str,
        }),
        Some("boolean") => Ok(Shape::Bool),
        Some("integer") => Ok(Shape::Int),
        Some("array") => {
            let item = Box::new(shape_from_schema(schema.get("items").ok_or("array without `items`")?)?);
            Ok(match schema.get("maxItems").and_then(Value::as_u64) {
                Some(max) => Shape::ListMax(item, max as usize),
                None => Shape::List(item),
            })
        }
        Some("object") => {
            let properties = schema.get("properties").and_then(Value::as_object).ok_or("object without `properties`")?;
            let required: Vec<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .ok_or("object without `required` (it gives the property order)")?
                .iter()
                .map(|value| value.as_str().ok_or("`required` entries must be strings"))
                .collect::<Result<_, _>>()?;
            if required.len() != properties.len() || required.iter().any(|name| !properties.contains_key(*name)) {
                return Err("`required` must list every property exactly once".into());
            }
            let mut fields = Vec::new();
            for name in required {
                let shape = shape_from_schema(&properties[name]).map_err(|error| format!("{name}: {error}"))?;
                fields.push((name.to_owned(), shape));
            }
            Ok(Shape::Obj(fields))
        }
        other => Err(format!("unsupported schema type {other:?}")),
    }
}

/// A GBNF string literal for `text`.
fn literal(text: &str) -> String {
    let mut out = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// GBNF for a shape: JSON with the properties in the shape's order. The `space` rule bounds the
/// indentation, so whitespace cannot run away.
pub fn grammar(shape: &Shape) -> String {
    let mut rules: Vec<String> = Vec::new();
    let root = rule_for(shape, &mut rules);
    let mut out = format!("root ::= {root}\n");
    for rule in rules {
        out.push_str(&rule);
        out.push('\n');
    }
    out.push_str("space ::= | \" \" | \"\\n\"{1,2} [ \\t]{0,20}\n");
    out.push_str("string ::= \"\\\"\" char* \"\\\"\" space\n");
    out.push_str("char ::= [^\"\\\\\\x7F\\x00-\\x1F] | [\\\\] ([\"\\\\bfnrt] | \"u\" [0-9a-fA-F]{4})\n");
    out.push_str("boolean ::= (\"true\" | \"false\") space\n");
    out.push_str("integer ::= [0-9]{1,7} space\n");
    out
}

fn rule_for(shape: &Shape, rules: &mut Vec<String>) -> String {
    match shape {
        Shape::Str => "string".to_owned(),
        Shape::Bool => "boolean".to_owned(),
        Shape::Int => "integer".to_owned(),
        Shape::StrMax(max) => {
            let name = format!("text{max}");
            let rule = format!("{name} ::= \"\\\"\" char{{0,{max}}} \"\\\"\" space");
            if !rules.contains(&rule) {
                rules.push(rule);
            }
            name
        }
        Shape::OneOf(values) => {
            let name = format!("enum{}", rules.len());
            let alternatives: Vec<String> = values.iter().map(|value| literal(&Value::String(value.clone()).to_string())).collect();
            rules.push(format!("{name} ::= ({}) space", alternatives.join(" | ")));
            name
        }
        Shape::List(item) => {
            let item_rule = rule_for(item, rules);
            let name = format!("list{}", rules.len());
            rules.push(format!("{name} ::= \"[\" space ({item_rule} (\",\" space {item_rule})*)? \"]\" space"));
            name
        }
        Shape::ListMax(_, 0) => {
            let rule = "none ::= \"[\" space \"]\" space".to_owned();
            if !rules.contains(&rule) {
                rules.push(rule);
            }
            "none".to_owned()
        }
        Shape::ListMax(item, max) => {
            let item_rule = rule_for(item, rules);
            let name = format!("list{}", rules.len());
            let more = max - 1;
            rules.push(format!("{name} ::= \"[\" space ({item_rule} (\",\" space {item_rule}){{0,{more}}})? \"]\" space"));
            name
        }
        Shape::Obj(properties) => {
            let fields: Vec<String> = properties
                .iter()
                .map(|(key, value)| {
                    let rule = rule_for(value, rules);
                    format!("{} space \":\" space {rule}", literal(&format!("\"{key}\"")))
                })
                .collect();
            let name = format!("obj{}", rules.len());
            rules.push(format!("{name} ::= \"{{\" space {} \"}}\" space", fields.join(" \",\" space ")));
            name
        }
    }
}
