use rmcp::model::Tool;

use super::PKG_VERSION;
use super::cache::load_cache;

/// Print help for a single tool, using cached tool metadata.
///
/// Returns `true` if the tool was found, `false` if unknown.
pub(super) fn print_tool_help(mcp_name: &str) -> bool {
    let cli_name = mcp_name.replace('_', "-");

    let tools = load_cache()
        .filter(|c| c.version == PKG_VERSION)
        .map(|c| c.tools)
        .unwrap_or_default();
    let tool = tools.iter().find(|t| *t.name == *mcp_name);

    match tool {
        Some(tool) => {
            let desc = tool
                .description
                .as_deref()
                .unwrap_or("No description available");
            println!("{cli_name} -- {desc}");
            println!();
            println!("Usage: det-cli {cli_name} [key=value ...]");

            if let Some(props) = tool.input_schema.get("properties")
                && let Some(obj) = props.as_object()
                && !obj.is_empty()
            {
                let required: Vec<&str> = tool
                    .input_schema
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();

                println!();
                println!("Parameters:");
                for (name, schema) in obj {
                    let pdesc = schema
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("");
                    let cli_param = name.replace('_', "-");
                    let req = if required.contains(&name.as_str()) {
                        " (required)"
                    } else {
                        ""
                    };
                    println!("  {:<28} {}{}", cli_param, pdesc, req);
                }
            } else {
                println!();
                println!("This command takes no parameters.");
            }
            true
        }
        None => {
            eprintln!(
                "Unknown tool '{cli_name}'. Run 'det-cli tools' to refresh the command list."
            );
            false
        }
    }
}

/// Print help output with integrated tool list.
/// When `tools` is `Some`, uses the provided list (live from server).
/// When `None`, falls back to the disk cache.
pub(super) fn print_help(tools: Option<&[Tool]>) {
    println!("det-cli -- Command-line interface for Dash Evo Tool (v{PKG_VERSION})");
    println!();
    println!("Usage: det-cli [command] [key=value ...]");

    // Tools section -- the main content.
    let cached;
    let tool_list: Option<&[Tool]> = match tools {
        Some(t) => Some(t),
        None => match load_cache() {
            Some(cache) if cache.version == PKG_VERSION => {
                cached = cache.tools;
                Some(&cached)
            }
            _ => None,
        },
    };

    println!();
    println!("Commands:");
    match tool_list {
        Some(tools) => {
            for tool in tools {
                let desc = tool.description.as_deref().unwrap_or("");
                let cli_name = tool.name.replace('_', "-");
                println!("  {:<30} {}", cli_name, desc);

                if let Some(props) = tool.input_schema.get("properties")
                    && let Some(obj) = props.as_object()
                {
                    for (name, schema) in obj {
                        let pdesc = schema
                            .get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or("");
                        let cli_param = name.replace('_', "-");
                        println!("    {:<28} {}", cli_param, pdesc);
                    }
                }
            }
        }
        None => {
            println!("  (run 'det-cli' once to discover available commands)");
        }
    }

    let help_line = |name: &str, desc: &str| println!("  {name:<30} {desc}");

    println!();
    println!("Management:");
    help_line("tools", "Refresh and display available commands");
    help_line("serve", "Run as MCP stdio server for AI agents");
    #[cfg(feature = "headless")]
    help_line("headless", "Run as headless HTTP MCP server daemon");
    help_line(
        "completion <shell>",
        "Generate shell completion (bash, zsh)",
    );
    println!();
    println!("Options:");
    help_line(
        "-s, --standalone",
        "Force standalone mode even when MCP_API_KEY is set",
    );
    help_line(
        "-a, --addr <url>",
        "Dash Evo Tool GUI address [env: MCP_LISTEN]",
    );
    help_line(
        "-b, --bearer <key>",
        "Bearer token for HTTP auth [env: MCP_API_KEY]",
    );
    help_line("-h, --help", "Print help");
    help_line("-V, --version", "Print version");
    println!();
    println!("By default, det-cli runs standalone using the last network from the GUI.");
    println!("Set MCP_API_KEY to connect to a running Dash Evo Tool instance.");
    println!("See docs/CLI.md for details.");
}

pub(super) fn print_result(result: &rmcp::model::CallToolResult) {
    use rmcp::model::RawContent;

    for content in &result.content {
        match &content.raw {
            RawContent::Text(text) => {
                println!("{}", text.text);
            }
            other => {
                println!("{:?}", other);
            }
        }
    }
    if result.is_error == Some(true) {
        std::process::exit(1);
    }
}

pub(super) fn parse_params(
    params: &[String],
) -> Result<serde_json::Map<String, serde_json::Value>, Box<dyn std::error::Error>> {
    let mut map = serde_json::Map::new();
    for param in params {
        let (key, value) = param
            .split_once('=')
            .ok_or_else(|| format!("Invalid parameter '{param}': expected key=value format"))?;
        let json_value = serde_json::from_str(value)
            .unwrap_or_else(|_| serde_json::Value::String(value.to_string()));
        map.insert(key.replace('-', "_"), json_value);
    }
    Ok(map)
}
