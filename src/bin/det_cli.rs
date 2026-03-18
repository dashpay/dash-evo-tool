//! det-cli -- Command-line client for Dash Evo Tool's MCP server.
//!
//! Connects to the MCP server, discovers tools dynamically, and calls them.
//! Supports HTTP transport (when MCP_API_KEY is configured) and stdio mode
//! (spawns dash-evo-tool-mcp as a child process).

use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};
use rmcp::model::{CallToolRequestParams, CallToolResult, RawContent, Tool};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};

type McpClient = RunningService<RoleClient, ()>;

#[derive(Parser)]
#[command(
    name = "det-cli",
    about = "CLI client for Dash Evo Tool (default: list tools)"
)]
struct Cli {
    /// Force stdio mode: spawn dash-evo-tool-mcp as a child process.
    /// Default when MCP_API_KEY is not configured.
    #[arg(long)]
    standalone: bool,

    /// HTTP server address (HTTP mode only).
    /// Defaults to http://{MCP_LISTEN}/mcp, or http://127.0.0.1:9527/mcp if MCP_LISTEN is unset.
    #[arg(long)]
    addr: Option<String>,

    /// Bearer token for HTTP auth. Reads MCP_API_KEY from env/.env automatically.
    #[arg(long, env = "MCP_API_KEY")]
    bearer: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// List available tools from the MCP server
    Tools,
    /// Call a tool by name with key=value parameters
    Call {
        /// Tool name
        tool: String,
        /// Parameters as key=value pairs (values parsed as JSON, falling back to string)
        #[arg(trailing_var_arg = true)]
        params: Vec<String>,
    },
    /// Cache tool schemas locally for shell completion
    Cache,
    /// Generate shell completion script
    Completion {
        /// Shell type
        shell: clap_complete::Shell,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load app's .env before clap reads env vars — won't override vars already set in the shell.
    load_app_env();

    // Intercept --help / -h to append cached tool list to clap's output.
    let args: Vec<String> = std::env::args().collect();
    if args.len() <= 1
        || (args.len() == 2 && (args[1] == "--help" || args[1] == "-h" || args[1] == "help"))
    {
        Cli::command().print_help()?;
        println!();
        print_cached_tools_help();
        return Ok(());
    }

    let cli = Cli::parse();

    if let Some(Commands::Completion { shell }) = &cli.command {
        generate_completion(*shell);
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;

    runtime.block_on(run(cli))
}

/// Load the app's `.env` file into the process environment.
/// Variables already set in the shell are not overridden.
fn load_app_env() {
    if let Ok(data_dir) = dash_evo_tool::app_dir::app_user_data_dir_path() {
        let env_path = data_dir.join(".env");
        if env_path.exists() {
            let _ = dotenvy::from_path(&env_path);
        }
    }
}

/// Print tool list from cache as a help appendix, or a hint to populate the cache.
fn print_cached_tools_help() {
    match load_cached_tools() {
        Ok(tools) if !tools.is_empty() => {
            println!("\nAvailable tools (cached — run 'det-cli cache' to refresh):");
            for tool in &tools {
                let desc = tool.description.as_deref().unwrap_or("");
                println!("  {:<30} {}", tool.name, desc);
            }
        }
        _ => {
            println!(
                "\nRun 'det-cli cache' to populate the tool list, then 'det-cli --help' to see available tools."
            );
        }
    }
}

/// Determine connection mode and run the requested command.
///
/// Uses stdio mode when `--standalone` is set or no bearer token is available.
/// Uses HTTP mode when a bearer token is configured (via flag, env, or .env).
async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let use_stdio = cli.standalone || cli.bearer.is_none();

    let client: McpClient = if use_stdio {
        connect_standalone().await?
    } else {
        let addr = resolve_addr(cli.addr);
        connect_http(&addr, cli.bearer.as_deref()).await?
    };

    match cli.command.unwrap_or(Commands::Tools) {
        Commands::Tools => {
            let tools = client.peer().list_all_tools().await?;
            print_tools(&tools);
        }
        Commands::Call { tool, params } => {
            let arguments = parse_params(&params)?;
            let mut request = CallToolRequestParams::new(tool);
            if !arguments.is_empty() {
                request.arguments = Some(arguments);
            }
            let result = client.peer().call_tool(request).await?;
            print_result(&result);
        }
        Commands::Cache => {
            let tools = client.peer().list_all_tools().await?;
            save_cache(&tools)?;
            eprintln!("Cached {} tools", tools.len());
        }
        Commands::Completion { .. } => unreachable!(),
    }

    Ok(())
}

/// Resolve the MCP server URL from the explicit flag, MCP_LISTEN env var, or the default.
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

async fn connect_standalone() -> Result<McpClient, Box<dyn std::error::Error>> {
    use rmcp::transport::child_process::TokioChildProcess;

    let transport = TokioChildProcess::new(tokio::process::Command::new("dash-evo-tool-mcp"))?;
    let client = ().serve(transport).await?;
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

fn print_tools(tools: &[Tool]) {
    for tool in tools {
        let desc = tool.description.as_deref().unwrap_or("");
        println!("{:<30} {}", tool.name, desc);

        if let Some(props) = tool.input_schema.get("properties")
            && let Some(obj) = props.as_object()
        {
            for (name, schema) in obj {
                let desc = schema
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");
                println!("  {:<26} {}", name, desc);
            }
        }
    }
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
        map.insert(key.to_string(), json_value);
    }
    Ok(map)
}

// -- Cache --

fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("org", "dash", "det-cli")
        .map(|p| p.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".det-cli-cache"))
}

fn cache_path() -> PathBuf {
    cache_dir().join("tools.json")
}

fn load_cached_tools() -> Result<Vec<Tool>, Box<dyn std::error::Error>> {
    let json = std::fs::read_to_string(cache_path())?;
    Ok(serde_json::from_str(&json)?)
}

fn save_cache(tools: &[Tool]) -> Result<(), Box<dyn std::error::Error>> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(tools)?;
    std::fs::write(cache_path(), json)?;
    Ok(())
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
        COMPREPLY+=( $(compgen -W "$(jq -r '.[].name' "$cache" 2>/dev/null)" -- "${{COMP_WORDS[COMP_CWORD]}}") )
    fi
}}
complete -F _det_cli_tools det-cli call
"#,
            cache = cache.display()
        );
    }
}
