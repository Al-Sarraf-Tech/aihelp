use std::io::{IsTerminal, Read};

use anyhow::{Context, Result};

pub const SYSTEM_PROMPT: &str = "You are a CLI helper. Ground answers ONLY in provided stdin context + general shell knowledge + MCP tool outputs (if MCP enabled). Never claim you executed commands. If asked 'what is in this directory?', interpret stdin as ls output. If uncertain, state what is unknown and suggest next read-only commands to run.";

#[derive(Debug, Clone)]
pub struct StdinContext {
    pub content: String,
    pub truncated: bool,
    pub bytes_read: usize,
    pub max_bytes: usize,
}

pub fn read_stdin_context(max_bytes: usize) -> Result<Option<StdinContext>> {
    if std::io::stdin().is_terminal() {
        return Ok(None);
    }

    let mut buf = Vec::new();
    let mut handle = std::io::stdin()
        .lock()
        .take((max_bytes as u64).saturating_add(1));
    handle
        .read_to_end(&mut buf)
        .context("failed to read stdin context")?;

    let (truncated_buf, truncated) = truncate_stdin_bytes(&buf, max_bytes);
    buf = truncated_buf;

    let bytes_read = buf.len();
    let content = String::from_utf8_lossy(&buf).to_string();

    Ok(Some(StdinContext {
        content,
        truncated,
        bytes_read,
        max_bytes,
    }))
}

pub fn truncate_stdin_bytes(input: &[u8], max_bytes: usize) -> (Vec<u8>, bool) {
    if input.len() <= max_bytes {
        return (input.to_vec(), false);
    }
    // Walk backwards from the cut-point to avoid splitting a multi-byte
    // UTF-8 sequence, which would produce replacement characters.
    let mut cut = max_bytes;
    while cut > 0 && (input[cut] & 0xC0) == 0x80 {
        cut -= 1;
    }
    (input[..cut].to_vec(), true)
}

pub fn build_user_message(question: &str, stdin_context: Option<&StdinContext>) -> String {
    let mut msg = String::new();
    msg.push_str("Question:\n");
    msg.push_str(question);
    msg.push_str("\n\n");

    if let Some(ctx) = stdin_context {
        msg.push_str("Context (stdin):\n");
        msg.push_str("```text\n");
        msg.push_str(&ctx.content);
        if !ctx.content.ends_with('\n') {
            msg.push('\n');
        }
        msg.push_str("```\n");

        if ctx.truncated {
            msg.push_str(&format!(
                "\nNote: stdin was truncated to {} bytes (max {}).\n",
                ctx.bytes_read, ctx.max_bytes
            ));
        }
        msg.push('\n');
    }

    msg.push_str("Answer clearly. Prefer bullets. Suggest next read-only commands if needed.");
    msg
}
