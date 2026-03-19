use clap::CommandFactory;

/// Generate shell completion script and print to stdout.
pub(super) fn generate_completion(shell: clap_complete::Shell) {
    let mut cmd = super::Cli::command();
    clap_complete::generate(shell, &mut cmd, "det-cli", &mut std::io::stdout());

    if shell == clap_complete::Shell::Bash {
        let cache = super::cache::cache_path();
        println!(
            r#"
# Wrap clap's completion with dynamic tool/parameter completion from cache
_det_cli_wrapper() {{
    _det-cli "$@"
    local cache="{cache}"
    [ -f "$cache" ] || return
    if [ "$COMP_CWORD" -eq 1 ]; then
        COMPREPLY+=( $(compgen -W "$(jq -r '.tools[].name' "$cache" 2>/dev/null | tr '_' '-')" -- "${{COMP_WORDS[COMP_CWORD]}}") )
    elif [ "$COMP_CWORD" -ge 2 ]; then
        local tool_name="${{COMP_WORDS[1]}}"
        local mcp_name=$(echo "$tool_name" | tr '-' '_')
        local params=$(jq -r --arg t "$mcp_name" '.tools[] | select(.name == $t) | .inputSchema.properties // {{}} | keys[]' "$cache" 2>/dev/null | tr '_' '-')
        local cur="${{COMP_WORDS[COMP_CWORD]}}"
        if [[ "$cur" == *=* ]]; then
            return
        fi
        COMPREPLY+=( $(compgen -S '=' -W "$params" -- "$cur") )
        compopt -o nospace
    fi
}}
complete -F _det_cli_wrapper -o nosort -o bashdefault -o default det-cli
"#,
            cache = cache.display()
        );
    }
}

/// Install bash completion to the user-level completions directory.
/// Writes to `~/.local/share/bash-completion/completions/det-cli` so it is
/// auto-loaded by bash-completion 2.x on the next shell -- no .bashrc editing needed.
pub(super) fn install_bash_completion() {
    let Some(base) = directories::BaseDirs::new() else {
        return;
    };
    let comp_dir = base.data_local_dir().join("bash-completion/completions");
    if std::fs::create_dir_all(&comp_dir).is_err() {
        return;
    }

    let mut buf = Vec::new();
    let mut cmd = super::Cli::command();
    clap_complete::generate(clap_complete::Shell::Bash, &mut cmd, "det-cli", &mut buf);

    let cache = super::cache::cache_path();
    buf.extend_from_slice(
        format!(
            r#"
# Wrap clap's completion with dynamic tool/parameter completion from cache
_det_cli_wrapper() {{
    _det-cli "$@"
    local cache="{cache}"
    [ -f "$cache" ] || return
    if [ "$COMP_CWORD" -eq 1 ]; then
        COMPREPLY+=( $(compgen -W "$(jq -r '.tools[].name' "$cache" 2>/dev/null | tr '_' '-')" -- "${{COMP_WORDS[COMP_CWORD]}}") )
    elif [ "$COMP_CWORD" -ge 2 ]; then
        local tool_name="${{COMP_WORDS[1]}}"
        local mcp_name=$(echo "$tool_name" | tr '-' '_')
        local params=$(jq -r --arg t "$mcp_name" '.tools[] | select(.name == $t) | .inputSchema.properties // {{}} | keys[]' "$cache" 2>/dev/null | tr '_' '-')
        local cur="${{COMP_WORDS[COMP_CWORD]}}"
        if [[ "$cur" == *=* ]]; then
            return
        fi
        COMPREPLY+=( $(compgen -S '=' -W "$params" -- "$cur") )
        compopt -o nospace
    fi
}}
complete -F _det_cli_wrapper -o nosort -o bashdefault -o default det-cli
"#,
            cache = cache.display()
        )
        .as_bytes(),
    );

    let dest = comp_dir.join("det-cli");
    let _ = std::fs::write(&dest, buf);
}
