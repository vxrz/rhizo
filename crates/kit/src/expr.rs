//! Portable flow expressions: the tiny language `flow` conditions and
//! templates are written in, compiled to each runtime's own (jq for
//! Serverless Workflow, JSONata for Node-RED).
//!
//! - References are paths into the data a step receives: `total`,
//!   `customer.email`, `order-id`.
//! - Literals: numbers, `"strings"`, `true`, `false`, `null`.
//! - Comparisons `==` `!=` `<` `<=` `>` `>=`, then `not`, `and`, `or`, and
//!   parentheses. No function calls and no arithmetic.
//! - Templates are strings with `{{ expression }}` parts. A template that is
//!   exactly one `{{ … }}` passes the value with its type.

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Ref(Vec<String>),
    /// A number as written.
    Num(String),
    Str(String),
    Bool(bool),
    Null,
    Cmp(Box<Expr>, Op, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(String),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Num(String),
    Str(String),
    Op(Op),
    Open,
    Close,
    Dot,
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let cs: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        let two = |a: char, b: char| c == a && cs.get(i + 1) == Some(&b);
        if c.is_whitespace() {
            i += 1;
        } else if c == '(' || c == ')' || c == '.' {
            out.push(match c {
                '(' => Tok::Open,
                ')' => Tok::Close,
                _ => Tok::Dot,
            });
            i += 1;
        } else if two('=', '=') || two('!', '=') || two('<', '=') || two('>', '=') {
            out.push(Tok::Op(match c {
                '=' => Op::Eq,
                '!' => Op::Ne,
                '<' => Op::Le,
                _ => Op::Ge,
            }));
            i += 2;
        } else if c == '<' || c == '>' {
            out.push(Tok::Op(if c == '<' { Op::Lt } else { Op::Gt }));
            i += 1;
        } else if c == '"' {
            let mut s = String::new();
            i += 1;
            loop {
                match cs.get(i) {
                    None => return Err("a string is not closed: add `\"`".into()),
                    Some('"') => break,
                    Some('\\') if matches!(cs.get(i + 1), Some('"' | '\\')) => {
                        s.push(cs[i + 1]);
                        i += 2;
                    }
                    Some(&ch) => {
                        s.push(ch);
                        i += 1;
                    }
                }
            }
            i += 1;
            out.push(Tok::Str(s));
        } else if c.is_ascii_digit() || (c == '-' && cs.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let start = i;
            i += 1;
            while i < cs.len()
                && (cs[i].is_ascii_digit() || (cs[i] == '.' && cs.get(i + 1).is_some_and(char::is_ascii_digit)))
            {
                i += 1;
            }
            out.push(Tok::Num(cs[start..i].iter().collect()));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < cs.len() && (cs[i].is_ascii_alphanumeric() || cs[i] == '_' || cs[i] == '-') {
                i += 1;
            }
            out.push(Tok::Word(cs[start..i].iter().collect()));
        } else if c == '=' {
            return Err("`=` compares nothing: use `==`".into());
        } else {
            return Err(format!("unexpected `{c}`"));
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    at: usize,
    /// Open parentheses and `not`s: the parser recurses on each.
    depth: usize,
}

/// Deeper than this, an expression is refused rather than risking the stack.
const MAX_NESTING: usize = 64;

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            return Err(format!("nested more than {MAX_NESTING} levels deep"));
        }
        Ok(())
    }

    fn word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(x)) if x == w)
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut l = self.and()?;
        while self.word("or") {
            self.at += 1;
            l = Expr::Or(Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut l = self.not()?;
        while self.word("and") {
            self.at += 1;
            l = Expr::And(Box::new(l), Box::new(self.not()?));
        }
        Ok(l)
    }

    fn not(&mut self) -> Result<Expr, String> {
        if self.word("not") {
            self.at += 1;
            self.enter()?;
            let e = self.not()?;
            self.depth -= 1;
            return Ok(Expr::Not(Box::new(e)));
        }
        let l = self.atom()?;
        if let Some(Tok::Op(op)) = self.peek().cloned() {
            self.at += 1;
            return Ok(Expr::Cmp(Box::new(l), op, Box::new(self.atom()?)));
        }
        Ok(l)
    }

    fn atom(&mut self) -> Result<Expr, String> {
        let t = self.peek().cloned().ok_or("the expression ends too early")?;
        self.at += 1;
        Ok(match t {
            Tok::Open => {
                self.enter()?;
                let e = self.or()?;
                self.depth -= 1;
                if self.peek() != Some(&Tok::Close) {
                    return Err("a `(` is not closed".into());
                }
                self.at += 1;
                e
            }
            Tok::Num(n) => Expr::Num(n),
            Tok::Str(s) => Expr::Str(s),
            Tok::Word(w) => match w.as_str() {
                "true" => Expr::Bool(true),
                "false" => Expr::Bool(false),
                "null" => Expr::Null,
                "and" | "or" | "not" => return Err(format!("`{w}` needs something before it")),
                _ => {
                    let mut path = vec![w];
                    while self.peek() == Some(&Tok::Dot) {
                        self.at += 1;
                        match self.peek().cloned() {
                            Some(Tok::Word(seg)) => path.push(seg),
                            _ => return Err(format!("`{}.` needs a name after the dot", path.join("."))),
                        }
                        self.at += 1;
                    }
                    Expr::Ref(path)
                }
            },
            Tok::Op(_) => return Err("a comparison needs a value on its left".into()),
            Tok::Close => return Err("unexpected `)`".into()),
            Tok::Dot => return Err("unexpected `.`: refer to fields by name, like `customer.email`".into()),
        })
    }
}

/// Parses one expression.
pub fn parse(src: &str) -> Result<Expr, String> {
    let mut p = Parser { toks: lex(src)?, at: 0, depth: 0 };
    if p.toks.is_empty() {
        return Err("the expression is empty".into());
    }
    let e = p.or()?;
    match p.peek() {
        None => Ok(e),
        Some(t) => Err(format!("unexpected {} after a complete expression", describe(t))),
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Word(w) => format!("`{w}`"),
        Tok::Num(n) => format!("`{n}`"),
        Tok::Str(s) => format!("\"{s}\""),
        Tok::Op(_) => "a comparison".into(),
        Tok::Open => "`(`".into(),
        Tok::Close => "`)`".into(),
        Tok::Dot => "`.`".into(),
    }
}

/// Splits a template into text and `{{ expression }}` parts.
pub fn template(src: &str) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    let mut rest = src;
    while let Some(start) = rest.find("{{") {
        if start > 0 {
            parts.push(Part::Text(rest[..start].to_string()));
        }
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or("a `{{` is not closed: add `}}`")?;
        parts.push(Part::Expr(parse(&after[..end])?));
        rest = &after[end + 2..];
    }
    if !rest.is_empty() {
        parts.push(Part::Text(rest.to_string()));
    }
    Ok(parts)
}

fn simple(seg: &str) -> bool {
    seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn quote(s: &str) -> String {
    serde_json::to_string(s).expect("strings serialize")
}

impl Op {
    fn jq(self) -> &'static str {
        match self {
            Op::Eq => "==",
            Op::Ne => "!=",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
        }
    }

    fn jsonata(self) -> &'static str {
        if self == Op::Eq { "=" } else { self.jq() }
    }
}

impl Expr {
    /// As jq, against the data (`.`), without the `${ }` wrapper.
    pub fn jq(&self) -> String {
        match self {
            Expr::Ref(path) => {
                path.iter().map(|s| if simple(s) { format!(".{s}") } else { format!(".{}", quote(s)) }).collect()
            }
            Expr::Num(n) => n.clone(),
            Expr::Str(s) => quote(s),
            Expr::Bool(b) => b.to_string(),
            Expr::Null => "null".into(),
            Expr::Cmp(l, op, r) => format!("{} {} {}", l.jq(), op.jq(), r.jq()),
            Expr::And(l, r) => format!("({} and {})", l.jq(), r.jq()),
            Expr::Or(l, r) => format!("({} or {})", l.jq(), r.jq()),
            Expr::Not(e) => format!("({} | not)", e.jq()),
        }
    }

    /// As JSONata, against a Node-RED message (the data is `payload`).
    pub fn jsonata(&self) -> String {
        match self {
            Expr::Ref(path) => {
                let segs: Vec<String> =
                    path.iter().map(|s| if simple(s) { s.clone() } else { format!("`{s}`") }).collect();
                format!("payload.{}", segs.join("."))
            }
            Expr::Num(n) => n.clone(),
            Expr::Str(s) => quote(s),
            Expr::Bool(b) => b.to_string(),
            Expr::Null => "null".into(),
            // A missing field is `null` in jq but undefined in JSONata.
            Expr::Cmp(l, Op::Eq, r) if **r == Expr::Null => format!("$not($exists({0}) and {0} != null)", l.jsonata()),
            Expr::Cmp(l, Op::Ne, r) if **r == Expr::Null => format!("($exists({0}) and {0} != null)", l.jsonata()),
            Expr::Cmp(l, op, r) => format!("{} {} {}", l.jsonata(), op.jsonata(), r.jsonata()),
            Expr::And(l, r) => format!("({} and {})", l.jsonata(), r.jsonata()),
            Expr::Or(l, r) => format!("({} or {})", l.jsonata(), r.jsonata()),
            Expr::Not(e) => format!("$not({})", e.jsonata()),
        }
    }
}

/// A template as a jq expression (without `${ }`), or `None` if it is
/// plain text.
pub fn template_jq(parts: &[Part]) -> Option<String> {
    match parts {
        [Part::Expr(e)] => Some(e.jq()),
        _ if parts.iter().all(|p| matches!(p, Part::Text(_))) => None,
        _ => Some(
            parts
                .iter()
                .map(|p| match p {
                    Part::Text(t) => quote(t),
                    Part::Expr(e) => format!("({} | tostring)", e.jq()),
                })
                .collect::<Vec<_>>()
                .join(" + "),
        ),
    }
}

/// A template as a JSONata expression; plain text becomes a string literal.
pub fn template_jsonata(parts: &[Part]) -> String {
    match parts {
        [] => quote(""),
        [Part::Expr(e)] => e.jsonata(),
        _ => parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => quote(t),
                // `&` turns its operands into strings.
                Part::Expr(e) => e.jsonata(),
            })
            .collect::<Vec<_>>()
            .join(" & "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn both(src: &str) -> (String, String) {
        let e = parse(src).unwrap();
        (e.jq(), e.jsonata())
    }

    #[test]
    fn conditions_compile_to_jq_and_jsonata() {
        assert_eq!(both("total > 1000"), (".total > 1000".into(), "payload.total > 1000".into()));
        assert_eq!(
            both(r#"customer.country == "DE" and not (express or total <= -1.5)"#),
            (
                r#"(.customer.country == "DE" and ((.express or .total <= -1.5) | not))"#.into(),
                r#"(payload.customer.country = "DE" and $not((payload.express or payload.total <= -1.5)))"#.into()
            )
        );
        assert_eq!(
            both("order-id != null"),
            (r#"."order-id" != null"#.into(), "($exists(payload.`order-id`) and payload.`order-id` != null)".into())
        );
        assert_eq!(both("a == null").1, "$not($exists(payload.a) and payload.a != null)");
        assert_eq!(both(r#"note == "say \"hi\"""#).0, r#".note == "say \"hi\"""#);
    }

    #[test]
    fn bad_expressions_say_why() {
        assert_eq!(parse("total = 3").unwrap_err(), "`=` compares nothing: use `==`");
        assert_eq!(parse("").unwrap_err(), "the expression is empty");
        assert_eq!(parse("(a > 1").unwrap_err(), "a `(` is not closed");
        assert_eq!(parse("a > 1 b").unwrap_err(), "unexpected `b` after a complete expression");
        assert_eq!(parse("a.").unwrap_err(), "`a.` needs a name after the dot");
        assert_eq!(parse("a > ").unwrap_err(), "the expression ends too early");
        assert!(parse("price * 2").is_err());
    }

    #[test]
    fn templates() {
        let one = template("{{ total }}").unwrap();
        assert_eq!(template_jq(&one).unwrap(), ".total");
        assert_eq!(template_jsonata(&one), "payload.total");
        let mixed = template("Order {{ id }} placed").unwrap();
        assert_eq!(template_jq(&mixed).unwrap(), r#""Order " + (.id | tostring) + " placed""#);
        assert_eq!(template_jsonata(&mixed), r#""Order " & payload.id & " placed""#);
        let plain = template("hello").unwrap();
        assert_eq!(template_jq(&plain), None);
        assert_eq!(template_jsonata(&plain), r#""hello""#);
        assert!(template("{{ total ").is_err());
        assert!(template("{{ 1 + }}").is_err());
    }
}
