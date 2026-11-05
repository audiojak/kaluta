//! Granola, the meeting notes app, as a read-only source of meetings,
//! their notes and transcripts (spec §14.14): through Granola's hosted
//! MCP server (`https://mcp.granola.ai/mcp`, Streamable HTTP), signed in
//! with OAuth in the browser — discovery, dynamic client registration and
//! PKCE, with a loopback redirect, as the MCP authorization spec has it.
//! Nothing here writes to Granola.

pub mod mcp;
pub mod meetings;
pub mod oauth;

pub use mcp::GranolaMcp;
pub use meetings::{Meeting, MeetingPage, Participant, parse_meetings};
pub use oauth::{Pending, Tokens};

/// Granola's MCP server.
pub const MCP_URL: &str = "https://mcp.granola.ai/mcp";
