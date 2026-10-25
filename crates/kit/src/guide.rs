//! The authoring guide: how to write rhizo diagrams, for people and for
//! language models (`rhizo guide`, MCP `authoring_guide`). The prose lives in
//! `guide.md`; the kit catalog at the end is generated, so it always matches
//! the kits in this binary.

use serde_json::Value;

use crate::Kits;

impl Kits {
    pub fn guide(&self) -> String {
        let mut s = include_str!("../guide.md").to_string();
        for kit in self.iter() {
            let used = if kit.name == crate::IMPLICIT {
                "built in, no `use` needed".to_string()
            } else {
                format!("`use {} version=\"{}.{}\"`", kit.name, kit.version.major, kit.version.minor)
            };
            s += &format!("\n### `{}` {} ({used})\n\n", kit.name, kit.version);
            if let Some(d) = &kit.description {
                s += &format!("{d}.\n\n");
            }
            for t in kit.types.values() {
                let on = if t.frame { "frame type" } else { "node type" };
                s += &format!("- **`{}`** ({on}): {}", t.name, t.description.as_deref().unwrap_or(&t.label));
                if !t.parents.is_empty() {
                    s += &format!(
                        " Goes inside: {}.",
                        t.parents.iter().map(|p| format!("`{p}`")).collect::<Vec<_>>().join(", ")
                    );
                }
                let props = props(&t.schema);
                if !props.is_empty() {
                    s += &format!(" Props: {}.", props.join("; "));
                }
                if !t.ports.is_empty() {
                    let ports: Vec<String> = t
                        .ports
                        .iter()
                        .map(|p| {
                            format!(
                                "{} `{}`{}",
                                p.port.dir.name(),
                                p.port.name.as_str(),
                                if p.default { " (default)" } else { "" }
                            )
                        })
                        .collect();
                    s += &format!(" Ports: {}.", ports.join(", "));
                }
                s += "\n";
            }
        }
        s
    }
}

/// `name` with its type or allowed values, default and requiredness.
fn props(schema: &Value) -> Vec<String> {
    // Required always, or unless imported (`then` of an `if` on `origin`).
    let required: Vec<&str> = [&schema["required"], &schema["then"]["required"]]
        .into_iter()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let Some(props) = schema["properties"].as_object() else { return Vec::new() };
    props
        .iter()
        // Read-only props are written by `rhizo import`, not by authors.
        .filter(|(_, p)| p["readOnly"] != Value::Bool(true))
        .map(|(name, p)| {
            let kind = if let Some(values) = p["enum"].as_array() {
                values.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" | ")
            } else {
                match &p["type"] {
                    Value::String(t) if t == "array" => "list".to_string(),
                    Value::String(t) => t.clone(),
                    Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" or "),
                    _ => "any".to_string(),
                }
            };
            let mut out = format!("`{name}` ({kind}");
            if let Some(f) = p["format"].as_str() {
                out += &format!(", {f}");
            }
            if required.contains(&name.as_str()) {
                out += ", required";
            } else if !p["default"].is_null() {
                out += &format!(", default {}", p["default"]);
            }
            out += ")";
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::Kits;

    /// Every example in the guide loads cleanly and passes the kit checks,
    /// so the guide cannot drift from the language.
    #[test]
    fn guide_examples_are_valid() {
        let kits = Kits::builtin();
        let guide = kits.guide();
        let blocks: Vec<&str> = guide.split("```kdl\n").skip(1).filter_map(|b| b.split("```").next()).collect();
        assert!(blocks.len() >= 4);
        for b in blocks {
            let loaded = rhizo_lang::load(b, rhizo_lang::Syntax::Kdl);
            assert!(loaded.diagnostics.is_empty(), "{b}\n{:#?}", loaded.diagnostics);
            let findings = crate::check(&loaded.document, &kits);
            assert!(
                findings.is_empty(),
                "{b}\n{:#?}",
                findings.iter().map(|f| &f.diagnostic.message).collect::<Vec<_>>()
            );
        }
        assert!(guide.contains("- **`data.operation`** (node type)"));
        assert!(guide.contains("`method` (\"get\" | \"put\""));
        assert!(guide.contains("### `rhizo` 0.1.0 (built in, no `use` needed)"));
    }
}
