//! Reading a root module's files: which files OpenTofu would load, and
//! their blocks with every expression kept as written.

use std::path::Path;

use hcl_edit::Span;
use hcl_edit::expr::Expression;
use hcl_edit::structure::{Body, Structure};
use serde_json::{Map, Value};

/// A top-level block. `body` is the form the diagram stores: arguments as
/// their HCL source text, nested blocks as arrays of bodies under
/// `type "label"…` keys.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: String,
    pub labels: Vec<String>,
    pub body: Map<String, Value>,
    /// The file it is written in.
    pub file: String,
}

impl Block {
    /// `aws_s3_bucket.logs`, `data.aws_region.current`, `module.vpc`;
    /// `None` for blocks without an address.
    pub fn address(&self) -> Option<String> {
        match (self.kind.as_str(), self.labels.as_slice()) {
            ("resource", [t, n]) => Some(format!("{t}.{n}")),
            ("data" | "ephemeral", [t, n]) => Some(format!("{}.{t}.{n}", self.kind)),
            ("module", [n]) => Some(format!("module.{n}")),
            _ => None,
        }
    }

    /// What the report calls it: `variable "region"`.
    pub fn title(&self) -> String {
        let labels: Vec<String> = self.labels.iter().map(|l| label(l)).collect();
        std::iter::once(self.kind.clone()).chain(labels).collect::<Vec<_>>().join(" ")
    }

    /// An argument's value when it is a plain string literal.
    pub fn literal(&self, key: &str) -> Option<String> {
        literal(self.body.get(key)?.as_str()?)
    }
}

/// What a folder holds, as OpenTofu would load it.
#[derive(Debug, Default)]
pub struct Source {
    /// Configuration files read, sorted.
    pub files: Vec<String>,
    pub blocks: Vec<Block>,
    /// `.terraform.lock.hcl`, verbatim.
    pub lock: Option<String>,
    /// Variable files OpenTofu loads by itself (`terraform.tfvars`,
    /// `*.auto.tfvars`): inputs, possibly secret, never imported.
    pub inputs: Vec<String>,
    /// What else is there and is not imported, with why.
    pub skipped: Vec<(String, &'static str)>,
}

pub const LOCK: &str = ".terraform.lock.hcl";

fn is_state(name: &str) -> bool {
    name.ends_with(".tfstate") || name.contains(".tfstate.") || name.ends_with(".tfplan")
}

impl Source {
    /// Reads the configuration in `dir` (not its subfolders). Fails with
    /// every construct that would be lost; state files are never opened.
    pub fn read(dir: &Path) -> Result<Source, Vec<String>> {
        let mut names: Vec<(String, bool)> = Vec::new();
        let entries = std::fs::read_dir(dir).map_err(|e| vec![format!("cannot read {}: {e}", dir.display())])?;
        for e in entries {
            let e = e.map_err(|e| vec![format!("cannot read {}: {e}", dir.display())])?;
            let ty = e.file_type().map_err(|err| vec![format!("{}: {err}", e.path().display())])?;
            let name = e.file_name().to_string_lossy().into_owned();
            names.push((name, ty.is_file()));
        }
        names.sort();
        let mut src = Source::default();
        let mut refused = Vec::new();
        let mut config = Vec::new();
        for (name, is_file) in &names {
            let stem = name.trim_end_matches(".json").trim_end_matches(".tf").trim_end_matches(".tofu");
            let tf = name.ends_with(".tf") || name.ends_with(".tofu");
            let tf_json = name.ends_with(".tf.json") || name.ends_with(".tofu.json");
            if name == LOCK && *is_file {
                src.lock = Some(read(&dir.join(name)).map_err(|e| vec![e])?);
            } else if name.starts_with('.') {
                // .terraform, .git: tool folders, not configuration.
            } else if is_state(name) {
                src.skipped.push((name.clone(), "state or plan: never read"));
            } else if !is_file {
                src.skipped.push((name.clone(), "not a file of the root module"));
            } else if (tf || tf_json) && (stem == "override" || stem.ends_with("_override")) {
                refused.push(format!("{name}: override files merge into other blocks; they cannot be imported yet"));
            } else if tf_json {
                refused.push(format!("{name}: JSON configuration cannot be imported yet"));
            } else if name.ends_with(".tf") && names.iter().any(|(n, _)| *n == format!("{stem}.tofu")) {
                src.skipped.push((name.clone(), "OpenTofu reads the .tofu file of the same name instead"));
            } else if tf {
                config.push(name.clone());
            } else if name == "terraform.tfvars"
                || name == "terraform.tfvars.json"
                || name.ends_with(".auto.tfvars")
                || name.ends_with(".auto.tfvars.json")
            {
                src.inputs.push(name.clone());
            } else {
                src.skipped.push((name.clone(), "not configuration"));
            }
        }
        for name in config {
            let text = match read(&dir.join(&name)) {
                Ok(t) => t,
                Err(e) => {
                    refused.push(e);
                    continue;
                }
            };
            match parse(&name, &text) {
                Ok(blocks) => src.blocks.extend(blocks),
                Err(e) => refused.extend(e),
            }
            src.files.push(name);
        }
        for b in src.blocks.iter().filter(|b| b.kind == "module") {
            let source = b.literal("source").unwrap_or_default();
            if source.starts_with("./") || source.starts_with("../") || source.starts_with(".\\") {
                refused.push(format!(
                    "{}: local module source `{source}`: its code would not be in the diagram (local modules are not imported yet)",
                    b.title()
                ));
            }
        }
        if refused.is_empty() { Ok(src) } else { Err(refused) }
    }
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// The blocks of one file.
pub fn parse(file: &str, text: &str) -> Result<Vec<Block>, Vec<String>> {
    let body = hcl_edit::parser::parse_body(text).map_err(|e| vec![format!("{file}: {e}")])?;
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for s in body.iter() {
        match s {
            Structure::Block(b) => {
                let labels = b.labels.iter().map(|l| l.as_str().to_string()).collect();
                let block = Block { kind: b.ident.as_str().to_string(), labels, body: Map::new(), file: file.into() };
                match body_of(&b.body, text) {
                    Ok(body) => out.push(Block { body, ..block }),
                    Err(e) => errors.push(format!("{file}: {}: {e}", block.title())),
                }
            }
            Structure::Attribute(a) => errors.push(format!("{file}: argument `{}` outside any block", a.key.as_str())),
        }
    }
    if errors.is_empty() { Ok(out) } else { Err(errors) }
}

fn body_of(body: &Body, text: &str) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for s in body.iter() {
        match s {
            Structure::Attribute(a) => {
                let span = a.value.span().ok_or("an expression without a position")?;
                let raw = text.get(span).ok_or("an expression outside the file")?;
                if out.insert(a.key.as_str().to_string(), Value::String(raw.to_string())).is_some() {
                    return Err(format!("argument `{}` is set twice", a.key.as_str()));
                }
            }
            Structure::Block(b) => {
                let key = std::iter::once(b.ident.as_str().to_string())
                    .chain(b.labels.iter().map(|l| label(l.as_str())))
                    .collect::<Vec<_>>()
                    .join(" ");
                let inner = Value::Object(body_of(&b.body, text)?);
                match out.entry(key.clone()).or_insert_with(|| Value::Array(Vec::new())) {
                    Value::Array(blocks) => blocks.push(inner),
                    _ => return Err(format!("`{key}` is both an argument and a block")),
                }
            }
        }
    }
    Ok(out)
}

/// A block label as written back: a quoted string literal. Must match the
/// compiler's `label`.
pub fn label(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

/// The value of an expression that is a plain string literal.
pub fn literal(raw: &str) -> Option<String> {
    match hcl_edit::parser::parse_expr(raw).ok()? {
        Expression::String(s) => Some(s.value().clone()),
        _ => None,
    }
}

/// `text` without its `import` blocks and its `terraform { encryption }`
/// blocks. Both make `tofu plan` call the cloud (an import reads the
/// resource; a KMS key provider calls STS), so L1 plans both sides without
/// them; the structural comparison still covers them.
pub fn without_imports(text: &str) -> Option<String> {
    let body = hcl_edit::parser::parse_body(text).ok()?;
    let mut spans: Vec<_> = body
        .iter()
        .filter_map(Structure::as_block)
        .flat_map(|b| match b.ident.as_str() {
            "import" => vec![b],
            "terraform" => {
                b.body.iter().filter_map(Structure::as_block).filter(|e| e.ident.as_str() == "encryption").collect()
            }
            _ => vec![],
        })
        .filter_map(|b| b.span())
        .collect();
    if spans.is_empty() {
        return None;
    }
    spans.sort_by_key(|s| std::cmp::Reverse(s.start));
    let mut out = text.to_string();
    for s in spans {
        out.replace_range(s, "");
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions_are_kept_as_written() {
        let text = "resource \"aws_s3_bucket\" \"b\" {\n  bucket = \"x-${var.env}\" # name\n  tags = {\n    A = 1\n  }\n  policy = <<EOT\n{\"a\": 1}\nEOT\n  dynamic \"rule\" {\n    for_each = var.rules\n    content { id = rule.key }\n  }\n  rule { id = \"b\" }\n}\n";
        let b = &parse("main.tf", text).unwrap()[0];
        assert_eq!(b.address().as_deref(), Some("aws_s3_bucket.b"));
        let body = serde_json::to_value(&b.body).unwrap();
        assert_eq!(body["bucket"], "\"x-${var.env}\"");
        assert_eq!(body["tags"], "{\n    A = 1\n  }");
        assert_eq!(body["policy"], "<<EOT\n{\"a\": 1}\nEOT");
        assert_eq!(body["dynamic \"rule\""][0]["content"][0]["id"], "rule.key");
        assert_eq!(body["rule"][0]["id"], "\"b\"");
        assert_eq!(b.literal("bucket"), None);
    }

    #[test]
    fn import_blocks_are_cut_for_planning() {
        let text = "import {\n  to = aws_s3_bucket.b\n  id = \"b\"\n}\nresource \"aws_s3_bucket\" \"b\" {}\n";
        let out = without_imports(text).unwrap();
        assert_eq!(parse("m.tf", &out).unwrap().iter().map(|b| b.kind.as_str()).collect::<Vec<_>>(), ["resource"]);
    }

    #[test]
    fn state_encryption_is_cut_for_planning() {
        let text = "terraform {\n  required_version = \">= 1.8\"\n  encryption {\n    key_provider \"aws_kms\" \"k\" {\n      kms_key_id = \"x\"\n    }\n  }\n}\n";
        let out = without_imports(text).unwrap();
        assert!(!out.contains("encryption") && out.contains("required_version"), "{out}");
    }
}
