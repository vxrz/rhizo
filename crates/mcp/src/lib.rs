//! `rhizo mcp FILE`: a Model Context Protocol server over one document, so
//! agents can explore it (nodes, neighbors, types), check it, preview what
//! it compiles to, and propose changes for a person to apply. Built on the
//! official Rust SDK (`rmcp`), stdio transport.

mod tools;

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::Value;

pub use tools::{Rendered, Workspace};

const INSTRUCTIONS: &str = "This server exposes a rhizo diagram, or a project folder of \
diagrams that refer to each other (`import \"model.rhizo\" as=model`, then `model:order`): KDL \
text files of nodes, edges and frames that a person reads as pictures. In a project, node \
results name their `file`, and `render`, `write_document` and `edit_document` take a `file`. Use it to understand what the person wants \
built (`overview`, `list_nodes`, `get_node`, `neighbors`, and `usages` across files), to get your build instructions \
(`brief`: goal, decisions, open questions and ordered tasks) and to see the specs it compiles \
to (`compile_preview`). `render` returns the picture. Before writing rhizo text, read \
`authoring_guide` (and `samples` for worked examples); check text with `validate`. If writing is enabled (`write_document`, \
`edit_document`), the person's viewer updates live as you write. Otherwise, or for changes the \
person should approve, use `propose_change`.";

/// Tools that change the document; listed only with `--allow-write`.
const WRITE_TOOLS: [&str; 3] = ["write_document", "edit_document", "adopt_nodes"];

#[derive(Clone)]
pub struct Server {
    ws: Arc<Workspace>,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ListNodesArgs {
    /// A type (`data.entity`) or a kit namespace (`data`) to filter by.
    #[serde(rename = "type", default)]
    ty: Option<String>,
    /// Only nodes in this frame (frame id).
    #[serde(default)]
    frame: Option<String>,
    /// Only nodes with this tag.
    #[serde(default)]
    tag: Option<String>,
    /// Case-insensitive text to find in labels or ids.
    #[serde(default)]
    query: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct NodeArgs {
    /// Node id (see `list_nodes`).
    id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct NeighborsArgs {
    /// Node id to start from.
    id: String,
    /// Only edges of this kind: data, control, dependency or plain.
    #[serde(default)]
    kind: Option<String>,
    /// out (edges leaving), in (edges arriving) or both (default).
    #[serde(default)]
    direction: Option<String>,
    /// How many hops to follow, 1 to 5 (default 1).
    #[serde(default)]
    depth: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct UsagesArgs {
    /// Node id, as `list_nodes` gives it (or the id inside `file`).
    id: String,
    /// In a project folder: the file the node is defined in, relative (e.g. `model.rhizo`).
    #[serde(default)]
    file: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TypesArgs {
    /// A kit (`data`) or a type (`data.field`). Omit for all kits.
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PreviewArgs {
    /// A bundle path such as `specs/shop-api.openapi.yaml`. Omit for the file list.
    #[serde(default)]
    path: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SourceArgs {
    /// Complete rhizo text (KDL), starting with `rhizo version=1` or `rhizo version=2`.
    source: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct WriteArgs {
    /// Complete rhizo text (KDL), starting with `rhizo version=1` or `rhizo version=2`.
    source: String,
    /// In a project folder: which file, relative (e.g. `flows/checkout.rhizo`); created if new.
    #[serde(default)]
    file: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct AdoptArgs {
    /// Ids of imported infra nodes to adopt (their addresses, e.g. `module.receipts`).
    #[serde(default)]
    nodes: Vec<String>,
    /// Adopt every node that switches to rhizo's code without a change instead.
    #[serde(default)]
    all: bool,
    /// `.tfvars` files, relative to the served folder, for variables without defaults.
    #[serde(default)]
    var_files: Vec<String>,
    /// In a project folder: which file, relative.
    #[serde(default)]
    file: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct EditArgs {
    /// Exact text to replace; must occur exactly once in the file.
    old: String,
    /// Replacement text.
    new: String,
    /// In a project folder: which file, relative.
    #[serde(default)]
    file: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SamplesArgs {
    /// A sample's id, for its files; leave it out for the list.
    #[serde(default)]
    id: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct BriefArgs {
    /// true: the task graph as JSON (tasks.json). false (default): the Markdown brief.
    #[serde(default)]
    json: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct RenderArgs {
    /// png (default; an image you can look at) or svg (text).
    #[serde(default)]
    format: Option<String>,
    /// In a project folder: which file to draw, relative.
    #[serde(default)]
    file: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ProposeArgs {
    /// Exact text to replace in the file (must occur once). Empty: create the file with `new`.
    old: String,
    /// The replacement text (or the new file's text).
    new: String,
    /// Why the change is needed, in a sentence or two.
    rationale: String,
    /// In a project folder: which file, relative (e.g. `api.rhizo`).
    #[serde(default)]
    file: Option<String>,
}

/// Runs a tool, turning a panic (a bug) into an error reply instead of
/// taking the whole server down.
fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err("internal error in rhizo (a bug): this request could not be completed".into()))
}

fn text(result: Result<String, String>) -> Result<CallToolResult, McpError> {
    Ok(match result {
        Ok(t) => CallToolResult::success(vec![ContentBlock::text(t)]),
        Err(msg) => CallToolResult::error(vec![ContentBlock::text(msg)]),
    })
}

fn reply(result: Result<Value, String>) -> Result<CallToolResult, McpError> {
    Ok(match result {
        Ok(v) => CallToolResult::structured(v),
        Err(msg) => CallToolResult::error(vec![ContentBlock::text(msg)]),
    })
}

#[tool_router]
impl Server {
    /// `allow_write` lists the tools that write the document;
    /// `allow_providers` are OpenTofu providers the person allows besides
    /// rhizo's list (an agent cannot add any).
    pub fn new(path: PathBuf, allow_write: bool, allow_providers: Vec<String>) -> Self {
        let mut tool_router = Self::tool_router();
        if !allow_write {
            for t in WRITE_TOOLS {
                tool_router.remove_route(t);
            }
        }
        let mut ws = Workspace::new(path);
        ws.allow_providers = allow_providers;
        Server { ws: Arc::new(ws), tool_router }
    }

    #[tool(
        description = "How to write rhizo diagrams: syntax, layouts, typed kits and their props, goals and decisions, with examples. Read this before writing or editing rhizo text."
    )]
    async fn authoring_guide(&self) -> Result<CallToolResult, McpError> {
        text(Ok(self.ws.guide()))
    }

    #[tool(
        description = "Worked examples of rhizo diagrams (mindmaps, flows, data models, infrastructure on three clouds, a project): without an id the list, with an id the sample's files as text. Good models to copy from."
    )]
    async fn samples(&self, Parameters(a): Parameters<SamplesArgs>) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.samples(a.id.as_deref())))
    }

    #[tool(description = "Check rhizo text without saving it: every error and warning with line, column and a fix.")]
    async fn validate(&self, Parameters(a): Parameters<SourceArgs>) -> Result<CallToolResult, McpError> {
        reply(guarded(|| Ok(self.ws.validate(&a.source))))
    }

    #[tool(
        description = "The build brief: goal, decisions, open questions, generated specs and the ordered tasks to build what the diagram describes. This is your instruction set for implementing it."
    )]
    async fn brief(&self, Parameters(a): Parameters<BriefArgs>) -> Result<CallToolResult, McpError> {
        text(guarded(|| self.ws.brief(a.json)))
    }

    #[tool(
        description = "Draw the diagram as the person sees it: a PNG image (default) or SVG text. Use it to check that what you wrote reads well."
    )]
    async fn render(&self, Parameters(a): Parameters<RenderArgs>) -> Result<CallToolResult, McpError> {
        let png = a.format.as_deref() != Some("svg");
        Ok(match guarded(|| self.ws.render(png, a.file.as_deref())) {
            Ok(Rendered::Png(bytes)) => {
                use base64::Engine as _;
                let data = base64::engine::general_purpose::STANDARD.encode(bytes);
                CallToolResult::success(vec![ContentBlock::image(data, "image/png")])
            }
            Ok(Rendered::Svg(svg)) => CallToolResult::success(vec![ContentBlock::text(svg)]),
            Err(msg) => CallToolResult::error(vec![ContentBlock::text(msg)]),
        })
    }

    #[tool(
        description = "Replace the whole document with new rhizo text. The text is validated first and not written if it has errors. The person's viewer shows the result live."
    )]
    async fn write_document(&self, Parameters(a): Parameters<WriteArgs>) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.write_document(&a.source, a.file.as_deref())))
    }

    #[tool(
        description = "Change part of the document: replace `old` (exact text that occurs once) with `new`. Everything else stays byte for byte. Not written if the result has errors."
    )]
    async fn edit_document(&self, Parameters(a): Parameters<EditArgs>) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.edit_document(&a.old, &a.new, a.file.as_deref())))
    }

    #[tool(
        description = "Adopt imported infrastructure nodes: rhizo generates their code from now on (with `moved` blocks), so their props become editable. Runs OpenTofu offline (L1-adopt) and writes nothing unless every original resource is accounted for; the report lists each changed attribute and each new resource. Takes a few minutes. OpenTofu runs every provider the code requires, even mocked, so only rhizo's allow-list of official providers is installed; code that requires another provider is refused by name. Only the person can allow more, when starting the server (`rhizo mcp --allow-provider`); tell them which provider was refused."
    )]
    async fn adopt_nodes(&self, Parameters(a): Parameters<AdoptArgs>) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.adopt_nodes(&a.nodes, a.all, &a.var_files, a.file.as_deref())))
    }

    #[tool(description = "Title, kits, frames, node and edge counts, and how many nodes of each type.")]
    async fn overview(&self) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.overview()))
    }

    #[tool(
        description = "Find nodes by type or kit namespace, frame, tag or text. Returns id, label, type, frame and parent; at most 500."
    )]
    async fn list_nodes(&self, Parameters(a): Parameters<ListNodesArgs>) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.list_nodes(a.ty.as_deref(), a.frame.as_deref(), a.tag.as_deref(), a.query.as_deref())))
    }

    #[tool(
        description = "Everything about one node: props, its kit type, ports, note, link, tags, acceptance criteria, provenance, children, and incoming and outgoing edges."
    )]
    async fn get_node(&self, Parameters(a): Parameters<NodeArgs>) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.get_node(&a.id)))
    }

    #[tool(description = "Nodes and edges around a node, optionally by edge kind and direction, up to 5 hops.")]
    async fn neighbors(&self, Parameters(a): Parameters<NeighborsArgs>) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.neighbors(&a.id, a.kind.as_deref(), a.direction.as_deref(), a.depth)))
    }

    #[tool(
        description = "Where a node is used in other files of the project: each reference to it (`model:order`) and the edges touching those references, with file and line. Read-only."
    )]
    async fn usages(&self, Parameters(a): Parameters<UsagesArgs>) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.usages(&a.id, a.file.as_deref())))
    }

    #[tool(
        description = "Check the document: syntax, structure and kit rules (props against schemas, ports, containment). Each problem has a line, column and often a fix."
    )]
    async fn check(&self) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.check()))
    }

    #[tool(
        description = "Kit catalog: all kits and their types, one kit's types, or one type with its ports and props JSON Schema."
    )]
    async fn types(&self, Parameters(a): Parameters<TypesArgs>) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.types(a.name.as_deref())))
    }

    #[tool(
        description = "Compile the document in memory and list the bundle files (JSON Schema, OpenAPI, AsyncAPI, manifest), or return one file's contents. Writes nothing."
    )]
    async fn compile_preview(&self, Parameters(a): Parameters<PreviewArgs>) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.compile_preview(a.path.as_deref())))
    }

    #[tool(
        description = "Propose an exact change to a diagram file (replace `old` with `new`, or create a file) with a rationale. It is checked now and stored under .rhizo/proposals/ for a person to apply with one command; the document is not edited."
    )]
    async fn propose_change(&self, Parameters(a): Parameters<ProposeArgs>) -> Result<CallToolResult, McpError> {
        reply(guarded(|| self.ws.propose_change(a.file.as_deref(), &a.old, &a.new, &a.rationale)))
    }
}

// The filtered router in the field, not a fresh one: write tools stay hidden.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("rhizo", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// Serves `path` over stdin/stdout until the client disconnects.
pub fn serve_stdio(path: PathBuf, allow_write: bool, allow_providers: Vec<String>) -> std::io::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(async {
        let running = Server::new(path, allow_write, allow_providers)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(std::io::Error::other)?;
        running.waiting().await.map_err(std::io::Error::other)?;
        Ok(())
    })
}

#[cfg(test)]
mod tests;
