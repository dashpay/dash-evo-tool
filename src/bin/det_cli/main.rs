//! det-cli -- Command-line client for Dash Evo Tool's MCP server.
//!
//! Connects to the MCP server, discovers tools dynamically, and calls them.
//! Mode is selected automatically: HTTP when MCP_API_KEY is set, in-process otherwise.

use clap::{Parser, Subcommand};
use rmcp::RoleClient;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;

mod cache;
mod completion;
mod connect;
#[cfg(feature = "headless")]
mod headless;
mod help;

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
    #[cfg(feature = "headless")]
    /// Run as headless HTTP MCP server daemon
    Headless,
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
        help::print_help(None);
        return Ok(());
    }

    let cli = Cli::parse();

    if let Some(Commands::Completion { shell }) = &cli.command {
        completion::generate_completion(*shell);
        return Ok(());
    }

    if matches!(cli.command, Some(Commands::Serve)) {
        return connect::run_stdio_server();
    }

    #[cfg(feature = "headless")]
    if matches!(cli.command, Some(Commands::Headless)) {
        return headless::run_headless();
    }

    // Logging is off by default -- set RUST_LOG to enable (e.g. RUST_LOG=debug).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off")),
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
    // Mode selection: --standalone or no bearer -> stdio; bearer present -> HTTP.
    let use_stdio = cli.standalone || cli.bearer.is_none();

    let client: McpClient = if use_stdio {
        connect::connect_in_process()
            .await
            .map_err(|e| e.to_string())?
    } else {
        let addr = resolve_addr(cli.addr);
        connect::connect_http(&addr, cli.bearer.as_deref())
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
                .map_err(connect::format_service_error)?;
            cache::save_cache(&client, &tools);
            help::print_help(Some(&tools));
        }
        Commands::Tool(args) => {
            let tool_name = args.first().ok_or("tool name required".to_string())?;

            if args[1..].iter().any(|a| a == "--help" || a == "-h") {
                let mcp_name = tool_name.replace('-', "_");
                help::print_tool_help(&mcp_name);
                return Ok(());
            }

            let mcp_name = tool_name.replace('-', "_");
            let arguments = help::parse_params(&args[1..]).map_err(|e| e.to_string())?;
            let mut request = CallToolRequestParams::new(mcp_name);
            if !arguments.is_empty() {
                request.arguments = Some(arguments);
            }
            let result = client
                .peer()
                .call_tool(request)
                .await
                .map_err(connect::format_service_error)?;
            help::print_result(&result);
        }
        Commands::Serve | Commands::Completion { .. } => unreachable!(),
        #[cfg(feature = "headless")]
        Commands::Headless => unreachable!(),
    }

    Ok(())
}
