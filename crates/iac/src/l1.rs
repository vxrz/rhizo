//! L1, the offline proof (docs/design/iac-import.md): the compiled bundle
//! has the same blocks as the original, and OpenTofu plans both the same
//! against the same empty state with the same mocked providers.

use std::path::Path;

use rhizo_core::Document;
use rhizo_kit::Kits;
use serde_json::{Value, json};

use crate::Error;
use crate::hcl::{Block, Source};
use crate::tofu::Run;

/// What L1 compared.
pub struct Summary {
    pub blocks: usize,
    pub instances: usize,
    pub outputs: usize,
}

/// The parts of the plan that must be equal: everything planned and the
/// configuration as OpenTofu read it (which covers variable declarations
/// and `check` blocks; variable values are the same files on both sides).
const COMPARED: [&str; 4] = ["planned_values", "resource_changes", "output_changes", "configuration"];

/// Compiles `doc`, then checks its one imported environment against the
/// original root module in `original` (`src`, planned as `planned`).
pub fn verify(run: &Run, doc: &Document, original: &Path, src: &Source, planned: &Value) -> Result<Summary, Error> {
    let envs: Vec<_> = doc.frames.iter().filter(|f| rhizo_kit::infra::imported(f)).collect();
    let [env] = envs.as_slice() else {
        return Err(Error::Refused(vec![format!(
            "the diagram has {} imported environments; L1 compares exactly one",
            envs.len()
        )]));
    };
    let bundle = rhizo_compile::compile(doc, &Kits::builtin(), "import.rhizo", "").map_err(|findings| {
        Error::Refused(
            findings.into_iter().map(|f| format!("the diagram does not compile: {}", f.diagnostic.message)).collect(),
        )
    })?;
    let prefix = format!("{}/", rhizo_compile::environment_dir(env.id.as_str()));
    let dir = run.workdir("compiled")?;
    let io = |e: std::io::Error| Error::Io(e.to_string());
    for (path, f) in &bundle.files {
        if let Some(rel) = path.strip_prefix(&prefix) {
            std::fs::write(dir.join(rel), &f.contents).map_err(io)?;
        }
    }
    // Variable values are inputs: both sides plan with the same ones.
    for input in &src.inputs {
        std::fs::copy(original.join(input), dir.join(input)).map_err(io)?;
    }
    let compiled = Source::read(&dir).map_err(Error::Refused)?;
    same_blocks(src, &compiled)?;
    let b = run.plan(&dir, &compiled.blocks, "compiled bundle")?;
    let mut diffs = Vec::new();
    for key in COMPARED {
        diff(key, &normalize(&planned[key]), &normalize(&b[key]), &mut diffs);
    }
    if !diffs.is_empty() {
        return Err(Error::Mismatch(diffs));
    }
    Ok(Summary {
        blocks: src.blocks.len(),
        instances: planned["resource_changes"].as_array().map_or(0, Vec::len),
        outputs: planned["output_changes"].as_object().map_or(0, |o| o.len()),
    })
}

/// Every block of the original is in the compiled code with the same
/// arguments, expressions and nested blocks (in any file and order, but
/// nested blocks of one type in their order), and nothing else.
fn same_blocks(a: &Source, b: &Source) -> Result<(), Error> {
    let key = |x: &Block| sorted(&json!([x.kind, x.labels, x.body])).to_string();
    let mut compiled: Vec<(String, &Block)> = b.blocks.iter().map(|y| (key(y), y)).collect();
    let mut unmatched = Vec::new();
    for x in &a.blocks {
        let k = key(x);
        match compiled.iter().position(|(c, _)| *c == k) {
            Some(i) => drop(compiled.remove(i)),
            None => unmatched.push(x),
        }
    }
    let mut diffs = Vec::new();
    for x in unmatched {
        // The same block with other contents: say where they differ.
        match compiled.iter().position(|(_, y)| y.kind == x.kind && y.labels == x.labels) {
            Some(i) => {
                let (_, y) = compiled.remove(i);
                let body = |b: &Block| sorted(&Value::Object(b.body.clone()));
                diff(&format!("{} ({})", x.title(), x.file), &body(x), &body(y), &mut diffs);
            }
            None => diffs.push(format!("{} ({}): missing in the compiled code", x.title(), x.file)),
        }
    }
    for (_, y) in compiled {
        diffs.push(format!("{} ({}): only in the compiled code", y.title(), y.file));
    }
    if a.lock != b.lock {
        diffs.push(format!("{}: differs", crate::hcl::LOCK));
    }
    if diffs.is_empty() { Ok(()) } else { Err(Error::Mismatch(diffs)) }
}

/// `v` with the keys of every object sorted: argument order has no meaning
/// in HCL (and the diagram's text form writes simple values first).
fn sorted(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), sorted(&m[k]))).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        v => v.clone(),
    }
}

/// Lists of things with addresses, sorted by address.
fn normalize(v: &Value) -> Value {
    match v {
        Value::Array(items) => {
            let mut items: Vec<Value> = items.iter().map(normalize).collect();
            if items.iter().all(|i| i.get("address").is_some()) {
                items.sort_by(|x, y| address(x).cmp(address(y)));
            }
            Value::Array(items)
        }
        Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), normalize(v))).collect()),
        v => v.clone(),
    }
}

fn address(v: &Value) -> &str {
    v.get("address").and_then(Value::as_str).unwrap_or_default()
}

const MAX_DIFFS: usize = 25;

/// Where `a` and `b` differ, as `path: a ≠ b` lines.
fn diff(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    if a == b || out.len() >= MAX_DIFFS {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                diff(&format!("{path}.{k}"), x.get(k).unwrap_or(&Value::Null), y.get(k).unwrap_or(&Value::Null), out);
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                let at = if address(p).is_empty() { i.to_string() } else { address(p).to_string() };
                diff(&format!("{path}[{at}]"), p, q, out);
            }
        }
        _ => out.push(format!("{path}: original {} ≠ compiled {}", short(a), short(b))),
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 120 { s.chars().take(117).collect::<String>() + "…" } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffs_name_the_path() {
        let a = json!({"r": [{"address": "b", "v": 1}, {"address": "a", "v": 2}]});
        let b = json!({"r": [{"address": "a", "v": 2}, {"address": "b", "v": 3}]});
        let mut out = Vec::new();
        diff("plan", &normalize(&a), &normalize(&b), &mut out);
        assert_eq!(out, ["plan.r[b].v: original 1 ≠ compiled 3"]);
    }
}
