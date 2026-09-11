//! det-cli -- Command-line client for Dash Evo Tool's MCP server.
//!
//! Connects to the MCP server, discovers tools dynamically, and calls them.
//! Mode is selected automatically: HTTP when MCP_API_KEY is set, in-process otherwise.

use clap::{Parser, Subcommand};
use dash_evo_tool::context::SDK_THREAD_STACK_SIZE;
use rmcp::RoleClient;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;

mod cache;
mod completion;
mod connect;
#[cfg(feature = "headless")]
mod headless;
mod help;
mod password;

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
        connect::run_stdio_server();
    }

    #[cfg(feature = "headless")]
    if matches!(cli.command, Some(Commands::Headless)) {
        return headless::run_headless();
    }

    // Logging is off by default -- set RUST_LOG to enable (e.g. RUST_LOG=debug).
    // The cap keeps rmcp's raw request logging (tool arguments, secrets
    // included) out of the output whatever RUST_LOG says.
    {
        use tracing_subscriber::layer::SubscriberExt as _;
        use tracing_subscriber::util::SubscriberInitExt as _;
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off")),
            )
            .with_writer(std::io::stderr)
            .finish()
            .with(dash_evo_tool::logging::sensitive_target_cap())
            .try_init();
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(SDK_THREAD_STACK_SIZE) // 4 MiB stack size for each worker thread
        .enable_all()
        .build()?;

    let exit_code: i32 = match runtime.block_on(run(cli)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    };

    // Hard-exit: bypass Tokio runtime teardown to prevent coordinator OS threads
    // (identity-sync, platform-address-sync, shielded-sync) from panicking when
    // they poll `tokio::time::sleep` against a shutting-down timer wheel.
    // The tool result has already been printed by this point; any SQLite writes
    // issued before the tool returned are transaction-committed.
    // See `DashMcpService::shutdown_wallet_backend` for the full race analysis.
    use std::io::Write as _;
    let _ = std::io::stdout().lock().flush();
    let _ = std::io::stderr().lock().flush();
    // TODO(graceful-teardown): replace with normal return once WalletBackend::quiesce() joins coordinator threads.
    // Until then no SQLite connection is closed, so the WAL is never checkpointed on exit: a data
    // directory det-cli wrote last carries MB-sized -wal sidecars, which is what bloats migration
    // fixture archives (tests/migration-fixtures/README.md).
    std::process::exit(exit_code);
}

/// What a client-mode invocation does once connected.
enum Action {
    ListTools,
    CallTool(Box<CallToolRequestParams>),
}

/// Resolves a tool call's name and arguments before any connection is made —
/// including reading the password when a password flag names a source — so
/// bad input or an unsafe password file fails without booting the app.
///
/// `http_addr` is the server address in HTTP mode and `None` in-process; a
/// password is only sent over HTTP to a destination that keeps it private.
///
/// Returns `None` when the invocation only asked for the tool's help, which is
/// printed here.
fn prepare_tool_call(
    args: &[String],
    http_addr: Option<&str>,
) -> Result<Option<CallToolRequestParams>, String> {
    let tool_name = args.first().ok_or("tool name required".to_string())?;
    let mcp_name = tool_name.replace('-', "_");

    if args[1..].iter().any(|a| a == "--help" || a == "-h") {
        if !help::print_tool_help(&mcp_name) {
            Err(format!("Unknown tool '{tool_name}'"))?;
        }
        return Ok(None);
    }

    let mut params = args[1..].to_vec();
    let password_source =
        password::take_password_source(&mut params).map_err(|e| password::describe(&e))?;
    let mut arguments = help::parse_params(&params).map_err(|e| e.to_string())?;
    password::reject_inline_password(&arguments).map_err(|e| password::describe(&e))?;
    if let Some(source) = password_source {
        // Checked before reading, so a mistyped command never consumes stdin.
        // An unknown (uncached) tool is let through: the server is the
        // authority on its parameters.
        if help::cached_tool_takes_param(&mcp_name, password::PASSWORD_PARAM) == Some(false) {
            let error = password::PasswordArgError::ToolTakesNoPassword {
                tool: tool_name.clone(),
            };
            return Err(password::describe(&error));
        }
        if let Some(addr) = http_addr {
            password::ensure_safe_destination(addr).map_err(|e| password::describe(&e))?;
        }
        let secret = password::read_password(&source).map_err(|e| password::describe(&e))?;
        password::insert_password(&mut arguments, &secret);
    }

    let mut request = CallToolRequestParams::new(mcp_name);
    if !arguments.is_empty() {
        request.arguments = Some(arguments);
    }
    Ok(Some(request))
}

async fn run(cli: Cli) -> Result<(), String> {
    let Cli {
        standalone,
        addr,
        bearer,
        command,
    } = cli;

    // An empty/whitespace bearer means "no key": the default .env ships
    // `MCP_API_KEY=` (empty), which dotenvy sets as an empty string, so clap's
    // env binding yields `Some("")`. Treat that exactly like an unset key —
    // matching the HTTP server, which also disables auth on an empty key.
    let bearer = bearer.as_deref().map(str::trim).filter(|b| !b.is_empty());

    // Mode selection: --standalone or no bearer -> stdio; bearer present -> HTTP.
    let use_stdio = standalone || bearer.is_none();
    let http_addr = (!use_stdio).then(|| resolve_addr(addr));

    let action = match command.unwrap_or(Commands::Tools) {
        Commands::Tools => Action::ListTools,
        Commands::Tool(args) => match prepare_tool_call(&args, http_addr.as_deref())? {
            Some(request) => Action::CallTool(Box::new(request)),
            None => return Ok(()),
        },
        Commands::Serve | Commands::Completion { .. } => unreachable!(),
        #[cfg(feature = "headless")]
        Commands::Headless => unreachable!(),
    };

    let client: McpClient = match http_addr.as_deref() {
        None => connect::connect_in_process()
            .await
            .map_err(|e| e.to_string())?,
        Some(addr) => connect::connect_http(addr, bearer)
            .await
            .map_err(|e| e.to_string())?,
    };

    match action {
        Action::ListTools => {
            let tools = client
                .peer()
                .list_all_tools()
                .await
                .map_err(connect::format_service_error)?;
            cache::save_cache(&client, &tools);
            help::print_help(Some(&tools));
        }
        Action::CallTool(request) => {
            let result = client
                .peer()
                .call_tool(*request)
                .await
                .map_err(connect::format_service_error)?;
            help::print_result(&result);
        }
    }

    Ok(())
}
