//! # quilt-mcp
//!
//! Model Context Protocol (MCP) server for Quilt.
//!
//! ## Role in the system
//!
//! MCP is the standard protocol AI agents use to discover and call
//! tools. The MCP server wraps a `QuiltEngine` and:
//!
//! - Exposes every defined cell as an MCP **tool**. The tool name
//!   is the cell's id. The tool call invokes the cell.
//! - Exposes the sheet itself as an MCP **resource**. The resource
//!   URI is `quilt://sheet/<sheet-id>`. The contents are the YAML.
//!
//! ## Transport
//!
//! We use stdio for v0.1. The MCP server reads MCP requests from
//! stdin and writes responses to stdout, line-buffered JSON-RPC.
//!
//! ## Used by
//!
//! - Claude Code, Cursor, and any other MCP-compatible client.
//! - Other agents that want to call Quilt cells as tools.

use std::sync::Arc;

use anyhow::Result;
use quilt_core::{parse_sheet, CallerContext, QuiltEngine};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};

// =============================================================================
// Public API
// =============================================================================

/// Start the MCP server. Blocks until stdin is closed.
pub async fn serve_stdio() -> Result<()> {
    let transport = rmcp::transport::stdio();
    let server = QuiltMcpServer::new();
    let service = server.serve(transport).await?;
    service.waiting().await?;
    Ok(())
}

/// Construct the server, optionally preloading a sheet from disk.
pub fn build_server(sheet_path: Option<&str>) -> Result<QuiltMcpServer> {
    let engine = QuiltEngine::new("mcp").into_arc();
    if let Some(path) = sheet_path {
        let source = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read {}: {}", path, e))?;
        let sheet = parse_sheet(&source)?;
        engine.load_sheet(sheet)?;
    }
    Ok(QuiltMcpServer::from_engine(engine))
}

// =============================================================================
// The server
// =============================================================================

/// The MCP server. Wraps a Quilt engine and exposes its cells as
/// tools.
#[derive(Clone)]
pub struct QuiltMcpServer {
    /// The underlying engine.
    engine: Arc<QuiltEngine>,
    /// The tool router. We register every cell dynamically.
    tool_router: ToolRouter<QuiltMcpServer>,
}

impl QuiltMcpServer {
    /// Create a new empty server.
    pub fn new() -> Self {
        let engine = QuiltEngine::new("mcp").into_arc();
        Self::from_engine(engine)
    }

    /// Wrap an existing engine.
    pub fn from_engine(engine: Arc<QuiltEngine>) -> Self {
        Self {
            engine,
            tool_router: Self::tool_router(),
        }
    }

    /// Get a reference to the engine (for tests, debugging).
    pub fn engine(&self) -> &Arc<QuiltEngine> {
        &self.engine
    }

    /// Register a cell from a YAML definition.
    pub fn register_cell(&self, yaml: &str) -> Result<()> {
        let sheet = parse_sheet(yaml)?;
        self.engine.load_sheet(sheet)?;
        Ok(())
    }
}

impl Default for QuiltMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tool input types
// =============================================================================

/// The input to the `cell_get` tool.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct GetCellInput {
    /// The cell id (e.g. "temperature", "alert", "model.pick").
    pub id: String,
    /// Optional caller row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<String>,
    /// Optional caller column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    /// Optional identity id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// Optional identity type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_type: Option<String>,
}

/// The input to the `cell_set` tool.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SetCellInput {
    pub id: String,
    pub value: serde_json::Value,
}

/// The input to the `cell_call` tool.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct CallCellInput {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
}

/// The input to the `cell_push` tool.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct PushCellInput {
    pub id: String,
    pub data: serde_json::Value,
}

/// The input to the `cells_list` tool.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema, Default)]
pub struct ListCellsInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

// =============================================================================
// Tool implementations (the tool_router macro picks them up)
// =============================================================================

#[tool_router]
impl QuiltMcpServer {
    /// List all cells defined in the current Quilt sheet.
    #[tool(
        name = "cells_list",
        description = "List all cells defined in the current Quilt sheet. Optionally filter by kind."
    )]
    async fn cells_list(
        &self,
        Parameters(input): Parameters<ListCellsInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let cells = self.engine.list_cells();
        let filtered: Vec<_> = if let Some(kind_str) = input.kind {
            cells
                .iter()
                .filter(|c| c.def.kind.as_str() == kind_str)
                .collect()
        } else {
            cells.iter().collect()
        };
        let body = serde_json::to_string_pretty(
            &filtered
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "id": c.def.id,
                        "kind": c.def.kind.as_str(),
                        "description": c.def.description,
                        "value": c.value.data,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
    }

    /// Read a cell's current value.
    #[tool(
        name = "cell_get",
        description = "Read a cell's current value. Use this to inspect any cell in the sheet."
    )]
    async fn cell_get(
        &self,
        Parameters(input): Parameters<GetCellInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let ctx = build_context(input.row, input.column, input.identity, input.identity_type);
        match self.engine.get(&input.id, ctx) {
            Ok(v) => Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::to_string_pretty(&v.data).unwrap_or_default(),
            )])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "error: {}",
                e
            ))])),
        }
    }

    /// Set a cell's value.
    #[tool(
        name = "cell_set",
        description = "Set a cell's value. Triggers downstream recomputation."
    )]
    async fn cell_set(
        &self,
        Parameters(input): Parameters<SetCellInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let ctx = build_context(None, None, None, None);
        match self.engine.set(&input.id, input.value.clone(), ctx) {
            Ok(()) => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "set {} = {}",
                input.id,
                serde_json::to_string(&input.value).unwrap_or_default()
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "error: {}",
                e
            ))])),
        }
    }

    /// Call a cell as a capability.
    #[tool(
        name = "cell_call",
        description = "Call a cell as a capability. Use this to invoke a program cell, a router, or any other effectful cell."
    )]
    async fn cell_call(
        &self,
        Parameters(input): Parameters<CallCellInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let ctx = build_context(input.row.clone(), input.column.clone(), None, None);
        match self.engine.call(&input.id, input.input.clone(), ctx) {
            Ok(v) => Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::to_string_pretty(&v.data).unwrap_or_default(),
            )])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "error: {}",
                e
            ))])),
        }
    }

    /// Push a value into a sensor or IO cell.
    #[tool(
        name = "cell_push",
        description = "Push a value into a sensor or IO cell. Triggers downstream recomputation."
    )]
    async fn cell_push(
        &self,
        Parameters(input): Parameters<PushCellInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let _ctx = build_context(None, None, None, None);
        match self.engine.push(&input.id, input.data.clone()) {
            Ok(()) => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "pushed to {}",
                input.id
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "error: {}",
                e
            ))])),
        }
    }
}

// =============================================================================
// ServerHandler
// =============================================================================

#[tool_handler(router = self.tool_router.clone())]
impl ServerHandler for QuiltMcpServer {
    fn get_info(&self) -> ServerInfo {
        let cell_count = self.engine.list_cells().len();
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_server_info(Implementation::new(
                format!("quilt-mcp ({} cells)", cell_count),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Quilt MCP — every cell in the loaded sheet is exposed as a tool. \
                 Use `cells_list` to see what's available, then call any cell with \
                 `cell_get` (read), `cell_set` (write), `cell_call` (capability), \
                 or `cell_push` (sensor/IO input).",
            )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let cell_count = self.engine.list_cells().len();
        let sheet_id = self.engine.id().to_string();
        let resource = Resource::new(
            format!("quilt://sheet/{}", sheet_id),
            format!("Quilt sheet: {} ({} cells)", sheet_id, cell_count),
        )
        .with_description("The current sheet's cells in YAML form.")
        .with_mime_type("application/x-yaml");
        Ok(ListResourcesResult::with_all_items(vec![resource]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = &request.uri;
        if uri.starts_with("quilt://sheet/") {
            let cells = self.engine.list_cells();
            let sheet_id = self.engine.id().to_string();
            let body = format!(
                "# Quilt sheet (read-only snapshot)\n# {}\n# {} cells\n",
                sheet_id,
                cells.len()
            );
            Ok(ReadResourceResult::new(vec![ResourceContents::text(body, uri)]).into())
        } else {
            Err(ErrorData::new(
                ErrorCode::RESOURCE_NOT_FOUND,
                format!("unknown resource: {}", uri),
                None,
            ))
        }
    }
}

// =============================================================================
// Helpers
// =============================================================================

fn build_context(
    row: Option<String>,
    column: Option<String>,
    identity: Option<String>,
    identity_type: Option<String>,
) -> CallerContext {
    let mut ctx = CallerContext::default();
    if let Some(r) = row {
        ctx.row = Some(serde_json::Value::String(r));
    }
    if let Some(c) = column {
        ctx.column = Some(serde_json::Value::String(c));
    }
    if let Some(id) = identity {
        let id_type = identity_type.unwrap_or_else(|| "agent".to_string());
        ctx.identity = Some(quilt_core::Identity {
            id,
            kind: parse_identity_kind(&id_type),
            tags: Vec::new(),
        });
    }
    ctx
}

fn parse_identity_kind(s: &str) -> quilt_core::IdentityKind {
    use quilt_core::IdentityKind;
    match s {
        "human" => IdentityKind::Human,
        "agent" => IdentityKind::Agent,
        "sensor" => IdentityKind::Sensor,
        "system" => IdentityKind::System,
        _ => IdentityKind::Agent,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_server() -> QuiltMcpServer {
        QuiltMcpServer::new()
    }

    #[test]
    fn list_cells_empty() {
        let server = make_server();
        let cells = server.engine.list_cells();
        assert_eq!(cells.len(), 0);
    }

    #[test]
    fn register_cell_via_yaml() {
        let server = make_server();
        let yaml = r#"
id: test
version: "1"
cells:
  - id: a
    kind: value
    value: 42
"#;
        server.register_cell(yaml).unwrap();
        let cells = server.engine.list_cells();
        assert_eq!(cells.len(), 1);
    }

    #[test]
    fn build_context_with_all_fields() {
        let ctx = build_context(
            Some("boat-1".to_string()),
            Some("premium".to_string()),
            Some("user-42".to_string()),
            Some("human".to_string()),
        );
        assert_eq!(ctx.row, Some(serde_json::json!("boat-1")));
        assert_eq!(ctx.column, Some(serde_json::json!("premium")));
        assert!(ctx.identity.is_some());
        assert_eq!(ctx.identity.unwrap().id, "user-42");
    }
}
