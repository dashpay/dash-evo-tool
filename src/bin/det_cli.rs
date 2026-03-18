//! det-cli -- Command-line client for Dash Evo Tool's MCP server.
//!
//! Connects to the MCP server, discovers tools dynamically, and calls them.
//! Mode is selected automatically: HTTP when MCP_API_KEY is set, in-process otherwise.

use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};
use rmcp::model::{CallToolRequestParams, CallToolResult, RawContent, Tool};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};

type McpClient = RunningService<RoleClient, ()>;

const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(
    name = "det-cli",
    version,
    about = "Command-line interface for Dash Evo Tool",
    disable_help_subcommand = true
)]
struct Cli {
    /// Force standalone mode (no server connection needed)
    #[arg(short, long)]
    standalone: bool,

    /// Dash Evo Tool GUI address [env: MCP_LISTEN]
    #[arg(short, long)]
    addr: Option<String>,

    /// Bearer token for HTTP auth [env: MCP_API_KEY]
    #[arg(short, long, env = "MCP_API_KEY")]
    bearer: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Clone)]
enum Commands {
    /// List available tools from the MCP server
    Tools,
    /// Run as MCP stdio server (for Claude Desktop, AI agents, etc.)
    Serve,
    /// Generate shell completion script
    Completion {
        /// Shell type
        shell: clap_complete::Shell,
    },
    /// Call an MCP tool (catch-all for dynamic tool names)
    #[command(external_subcommand)]
    Tool(Vec<String>),
}

/// Resolve the HTTP address from CLI flag, env var, or default.
fn resolve_addr(addr: Option<String>) -> String {
    if let Some(a) = addr {
        return a;
    }
    if let Ok(listen) = std::env::var("MCP_LISTEN")
        && !listen.is_empty()
    {
        return format!("http://{listen}/mcp");
    }
    "http://127.0.0.1:9527/mcp".to_string()
}

/// Load the app's .env file. Shell env vars take precedence (dotenvy won't override).
fn load_app_env() {
    if let Ok(data_dir) = dash_evo_tool::app_dir::app_user_data_dir_path() {
        let env_path = data_dir.join(".env");
        if env_path.exists() {
            let _ = dotenvy::from_path(&env_path);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env before clap parses env vars (shell > .env > defaults).
    load_app_env();

    // Intercept --help to show custom help with tool list.
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && (args[1] == "--help" || args[1] == "-h") {
        print_help(None);
        return Ok(());
    }

    let cli = Cli::parse();

    if let Some(Commands::Completion { shell }) = &cli.command {
        generate_completion(*shell);
        return Ok(());
    }

    if matches!(cli.command, Some(Commands::Serve)) {
        return run_stdio_server();
    }

    // Minimal stderr logging: show warnings from our code but suppress rmcp
    // service-level warnings (they duplicate errors we already handle).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,rmcp=error")),
        )
        .with_writer(std::io::stderr)
        .try_init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;

    if let Err(e) = runtime.block_on(run(cli)) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
    Ok(())
}

async fn run(cli: Cli) -> Result<(), String> {
    // Mode selection: --standalone or no bearer → stdio; bearer present → HTTP.
    let use_stdio = cli.standalone || cli.bearer.is_none();

    let client: McpClient = if use_stdio {
        connect_in_process().await.map_err(|e| e.to_string())?
    } else {
        let addr = resolve_addr(cli.addr);
        connect_http(&addr, cli.bearer.as_deref())
            .await
            .map_err(|e| e.to_string())?
    };

    let command = cli.command.unwrap_or(Commands::Tools);
    match command {
        Commands::Tools => {
            let tools = client
                .peer()
                .list_all_tools()
                .await
                .map_err(format_service_error)?;
            save_cache(&client, &tools);
            print_help(Some(&tools));
        }
        Commands::Tool(args) => {
            let tool_name = args.first().ok_or("tool name required".to_string())?;

            if args[1..].iter().any(|a| a == "--help" || a == "-h") {
                let mcp_name = tool_name.replace('-', "_");
                print_tool_help(&mcp_name);
                return Ok(());
            }

            let mcp_name = tool_name.replace('-', "_");
            let arguments = parse_params(&args[1..]).map_err(|e| e.to_string())?;
            let mut request = CallToolRequestParams::new(mcp_name);
            if !arguments.is_empty() {
                request.arguments = Some(arguments);
            }
            let result = client
                .peer()
                .call_tool(request)
                .await
                .map_err(format_service_error)?;
            print_result(&result);
        }
        Commands::Serve | Commands::Completion { .. } => unreachable!(),
    }

    Ok(())
}

/// Format an rmcp ServiceError into a user-friendly string.
/// Extracts the message from McpError; passes other variants through as-is.
fn format_service_error(e: rmcp::service::ServiceError) -> String {
    use rmcp::service::ServiceError;
    match e {
        ServiceError::McpError(mcp) => format!("{}  (code {})", mcp.message, mcp.code.0),
        other => other.to_string(),
    }
}

/// Run as a standalone MCP stdio server (replaces the separate dash-evo-tool-mcp binary).
fn run_stdio_server() -> Result<(), Box<dyn std::error::Error>> {
    use dash_evo_tool::logging::initialize_logger;

    initialize_logger();
    tracing::info!(
        version = dash_evo_tool::VERSION,
        "Starting Dash Evo Tool MCP server (stdio)"
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;

    runtime
        .block_on(dash_evo_tool::mcp::start_stdio())
        .map_err(|e| -> Box<dyn std::error::Error> { e })
}

async fn connect_in_process() -> Result<McpClient, Box<dyn std::error::Error>> {
    use dash_evo_tool::mcp::server::DashMcpService;

    // Create two duplex byte channels, cross-connected:
    // client writes to a, server reads from a; server writes to b, client reads from b.
    let (client_read, server_write) = tokio::io::duplex(8192);
    let (server_read, client_write) = tokio::io::duplex(8192);

    // Spawn the MCP service in a background task.
    // .serve() returns a RunningService — keep it alive with .waiting().
    let service = DashMcpService::new_lazy();
    tokio::spawn(async move {
        match service.serve((server_read, server_write)).await {
            Ok(running) => {
                let _ = running.waiting().await;
            }
            Err(e) => eprintln!("MCP service error: {e}"),
        }
    });

    let client = ().serve((client_read, client_write)).await?;
    Ok(client)
}

async fn connect_http(
    addr: &str,
    bearer: Option<&str>,
) -> Result<McpClient, Box<dyn std::error::Error>> {
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };

    let config = StreamableHttpClientTransportConfig {
        uri: addr.into(),
        auth_header: bearer.map(|token| format!("Bearer {token}")),
        ..Default::default()
    };
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ().serve(transport).await?;
    Ok(client)
}

fn print_result(result: &CallToolResult) {
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

fn parse_params(
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

// -- Cache --

/// Versioned tool cache stored on disk.
#[derive(serde::Serialize, serde::Deserialize)]
struct ToolCache {
    version: String,
    tools: Vec<Tool>,
}

fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("org", "dash", "det-cli")
        .map(|p| p.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".det-cli-cache"))
}

fn cache_path() -> PathBuf {
    cache_dir().join("tools.json")
}

fn load_cache() -> Option<ToolCache> {
    let data = std::fs::read_to_string(cache_path()).ok()?;
    serde_json::from_str(&data).ok()
}

/// Save tools to cache and install shell completion.
/// Extracts server version from the peer info.
fn save_cache(client: &McpClient, tools: &[Tool]) {
    let version = client
        .peer()
        .peer_info()
        .map(|info| info.server_info.version.clone())
        .unwrap_or_else(|| PKG_VERSION.to_string());

    let cache = ToolCache {
        version,
        tools: tools.to_vec(),
    };

    let dir = cache_dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = serde_json::to_string_pretty(&cache).map(|json| std::fs::write(cache_path(), json));
    }

    install_bash_completion();
}

/// Install bash completion to the user-level completions directory.
/// Writes to `~/.local/share/bash-completion/completions/det-cli` so it is
/// auto-loaded by bash-completion 2.x on the next shell — no .bashrc editing needed.
fn install_bash_completion() {
    let Some(base) = directories::BaseDirs::new() else {
        return;
    };
    let comp_dir = base.data_local_dir().join("bash-completion/completions");
    if std::fs::create_dir_all(&comp_dir).is_err() {
        return;
    }

    let mut buf = Vec::new();
    let mut cmd = Cli::command();
    clap_complete::generate(clap_complete::Shell::Bash, &mut cmd, "det-cli", &mut buf);

    // Append dynamic tool name completion from cache.
    let cache = cache_path();
    buf.extend_from_slice(
        format!(
            r#"
# Dynamic tool name completion from cache
_det_cli_tools() {{
    local cache="{cache}"
    if [ -f "$cache" ]; then
        COMPREPLY+=( $(compgen -W "$(jq -r '.tools[].name' "$cache" 2>/dev/null | tr '_' '-')" -- "${{COMP_WORDS[COMP_CWORD]}}") )
    fi
}}
complete -F _det_cli_tools det-cli
"#,
            cache = cache.display()
        )
        .as_bytes(),
    );

    let dest = comp_dir.join("det-cli");
    let _ = std::fs::write(&dest, buf);
}

/// Print help for a single tool, using cached tool metadata.
fn print_tool_help(mcp_name: &str) {
    let cli_name = mcp_name.replace('_', "-");

    let tools = load_cache().map(|c| c.tools).unwrap_or_default();
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
        }
        None => {
            eprintln!(
                "Unknown tool '{cli_name}'. Run 'det-cli tools' to refresh the command list."
            );
        }
    }
}

/// Print help output with integrated tool list.
/// When `tools` is `Some`, uses the provided list (live from server).
/// When `None`, falls back to the disk cache.
fn print_help(tools: Option<&[Tool]>) {
    println!("det-cli — Command-line interface for Dash Evo Tool (v{PKG_VERSION})");
    println!();
    println!("Usage: det-cli [command] [key=value ...]");

    // Tools section — the main content.
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

// -- Completion --

fn generate_completion(shell: clap_complete::Shell) {
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "det-cli", &mut std::io::stdout());

    if shell == clap_complete::Shell::Bash {
        let cache = cache_path();
        println!(
            r#"
# Dynamic tool name completion from cache
_det_cli_tools() {{
    local cache="{cache}"
    if [ -f "$cache" ]; then
        COMPREPLY+=( $(compgen -W "$(jq -r '.tools[].name' "$cache" 2>/dev/null | tr '_' '-')" -- "${{COMP_WORDS[COMP_CWORD]}}") )
    fi
}}
complete -F _det_cli_tools det-cli
"#,
            cache = cache.display()
        );
    }
}
