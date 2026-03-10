use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const ZSH_COMPLETION: &str = r#"#compdef tg tracegrep

_tg() {
  local curcontext="$curcontext" state
  typeset -A opt_args

  _arguments -C \
    '--json[Output enriched results as JSON]' \
    '--compact[Collapse human-readable context onto the location line]' \
    '--build-index[Build or refresh the cached call graph index, then exit]' \
    '--include-tests[Include test-file callers and references in the graph]' \
    '--include-test-callers[Show callers that originate from test code]' \
    '--depth[How many caller levels to show]:depth:' \
    '--max-context[Max callers or references per section]:count:' \
    '--generate[Generate completion scripts]:target:(complete-bash complete-zsh complete-fish)' \
    '--install-completions[Install shell completions for the current or given shell]:shell:(bash zsh fish)' \
    '(-h --help)'{-h,--help}'[Print help]' \
    '(-V --version)'{-V,--version}'[Print version]' \
    '*:arg:->args'

  case $state in
    args)
      local saw_delim=0
      local build_index=0
      local positional=0
      local expect_value=0
      local arg

      for arg in "${words[@]:1}"; do
        if (( expect_value )); then
          expect_value=0
          continue
        fi
        if (( saw_delim )); then
          (( positional++ ))
          continue
        fi
        case "$arg" in
          --)
            saw_delim=1
            ;;
          --build-index)
            build_index=1
            ;;
          --depth|--max-context|--generate|--install-completions)
            expect_value=1
            ;;
          -*)
            ;;
          *)
            (( positional++ ))
            ;;
        esac
      done

      if (( build_index )); then
        _files
      elif (( positional >= 2 )); then
        _files
      else
        _message 'pattern'
      fi
      ;;
  esac
}

compdef _tg tg tracegrep
"#;

const BASH_COMPLETION: &str = r#"_tg() {
  local cur prev positional expect_value saw_delim build_index word i
  COMPREPLY=()
  cur="${COMP_WORDS[COMP_CWORD]}"
  prev="${COMP_WORDS[COMP_CWORD-1]}"

  local opts="--json --compact --build-index --include-tests --include-test-callers --depth --max-context --generate --install-completions -h --help -V --version"

  case "${prev}" in
    --generate)
      COMPREPLY=($(compgen -W "complete-bash complete-zsh complete-fish" -- "${cur}"))
      return 0
      ;;
    --install-completions)
      COMPREPLY=($(compgen -W "bash zsh fish" -- "${cur}"))
      return 0
      ;;
    --depth|--max-context)
      return 0
      ;;
  esac

  if [[ ${cur} == -* ]]; then
    COMPREPLY=($(compgen -W "${opts}" -- "${cur}"))
    return 0
  fi

  positional=0
  expect_value=0
  saw_delim=0
  build_index=0

  for ((i = 1; i < ${#COMP_WORDS[@]}; i++)); do
    word="${COMP_WORDS[i]}"
    if (( expect_value )); then
      expect_value=0
      continue
    fi
    if (( saw_delim )); then
      ((positional++))
      continue
    fi
    case "${word}" in
      --)
        saw_delim=1
        ;;
      --build-index)
        build_index=1
        ;;
      --depth|--max-context|--generate|--install-completions)
        expect_value=1
        ;;
      -*)
        ;;
      *)
        ((positional++))
        ;;
    esac
  done

  if (( build_index || positional >= 1 )); then
    COMPREPLY=($(compgen -f -- "${cur}"))
  fi
}

complete -F _tg tg
complete -F _tg tracegrep
"#;

const FISH_COMPLETION: &str = r#"function __tg_positional_state
    set -l tokens (commandline -opc)
    set -e tokens[1]

    set -l positional 0
    set -l expect_value 0
    set -l saw_delim 0
    set -l build_index 0

    for token in $tokens
        if test $expect_value -eq 1
            set expect_value 0
            continue
        end

        if test $saw_delim -eq 1
            set positional (math $positional + 1)
            continue
        end

        switch $token
            case --
                set saw_delim 1
            case --build-index
                set build_index 1
            case --depth --max-context --generate --install-completions
                set expect_value 1
            case '-*'
            case '*'
                set positional (math $positional + 1)
        end
    end

    echo $build_index $positional
end

function __tg_needs_path
    set -l state (__tg_positional_state)
    test $state[1] -eq 1; and return 0
    test $state[2] -ge 1
end

complete -c tg -c tracegrep -l json -d 'Output enriched results as JSON'
complete -c tg -c tracegrep -l compact -d 'Collapse human-readable context onto the location line'
complete -c tg -c tracegrep -l build-index -d 'Build or refresh the cached call graph index, then exit'
complete -c tg -c tracegrep -l include-tests -d 'Include test-file callers and references in the graph'
complete -c tg -c tracegrep -l include-test-callers -d 'Show callers that originate from test code'
complete -c tg -c tracegrep -l depth -d 'How many caller levels to show' -r
complete -c tg -c tracegrep -l max-context -d 'Max callers or references per section' -r
complete -c tg -c tracegrep -l generate -d 'Generate completion scripts' -r -a 'complete-bash complete-zsh complete-fish'
complete -c tg -c tracegrep -l install-completions -d 'Install shell completions' -r -a 'bash zsh fish'
complete -c tg -c tracegrep -s h -l help -d 'Print help'
complete -c tg -c tracegrep -s V -l version -d 'Print version'
complete -c tg -c tracegrep -f -n '__tg_needs_path' -a '(__fish_complete_path)'
"#;

const ZSH_RC_MARKER_START: &str = "# >>> tracegrep completions >>>";
const ZSH_RC_MARKER_END: &str = "# <<< tracegrep completions <<<";
const BASH_RC_MARKER_START: &str = "# >>> tracegrep completions >>>";
const BASH_RC_MARKER_END: &str = "# <<< tracegrep completions <<<";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "bash" => Ok(Self::Bash),
            "zsh" => Ok(Self::Zsh),
            "fish" => Ok(Self::Fish),
            other => anyhow::bail!("unsupported shell {other:?}; expected bash, zsh, or fish"),
        }
    }

    fn generate_arg(value: &str) -> anyhow::Result<Self> {
        match value {
            "complete-bash" => Ok(Self::Bash),
            "complete-zsh" => Ok(Self::Zsh),
            "complete-fish" => Ok(Self::Fish),
            other => anyhow::bail!(
                "unsupported generate target {other:?}; expected complete-bash, complete-zsh, or complete-fish"
            ),
        }
    }

    fn detect() -> anyhow::Result<Self> {
        let shell = env::var("SHELL")
            .map_err(|_| anyhow::anyhow!("SHELL is not set; pass an explicit shell"))?;
        let name = Path::new(&shell)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("failed to detect shell from SHELL={shell:?}"))?;
        Self::parse(name)
    }

    fn completion_text(self) -> &'static str {
        match self {
            Self::Bash => BASH_COMPLETION,
            Self::Zsh => ZSH_COMPLETION,
            Self::Fish => FISH_COMPLETION,
        }
    }
}

pub struct InstallResult {
    pub shell: Shell,
    pub written_files: Vec<PathBuf>,
    pub updated_rc_files: Vec<PathBuf>,
}

pub fn generate(target: &str) -> anyhow::Result<&'static str> {
    Ok(Shell::generate_arg(target)?.completion_text())
}

pub fn install(shell_arg: Option<&str>) -> anyhow::Result<InstallResult> {
    let shell = match shell_arg {
        Some(value) => Shell::parse(value)?,
        None => Shell::detect()?,
    };
    match shell {
        Shell::Bash => install_bash(),
        Shell::Zsh => install_zsh(),
        Shell::Fish => install_fish(),
    }
}

fn install_bash() -> anyhow::Result<InstallResult> {
    let completions_dir = data_home()?.join("bash-completion").join("completions");
    fs::create_dir_all(&completions_dir)?;
    let tg_path = completions_dir.join("tg");
    let tracegrep_path = completions_dir.join("tracegrep");
    write_file(&tg_path, BASH_COMPLETION)?;
    write_file(&tracegrep_path, BASH_COMPLETION)?;

    let bashrc = home_dir()?.join(".bashrc");
    ensure_managed_block(
        &bashrc,
        BASH_RC_MARKER_START,
        BASH_RC_MARKER_END,
        &format!(
            "if [ -f {path} ]; then\n  . {path}\nfi",
            path = shell_single_quote(&tg_path)
        ),
    )?;

    Ok(InstallResult {
        shell: Shell::Bash,
        written_files: vec![tg_path, tracegrep_path],
        updated_rc_files: vec![bashrc],
    })
}

fn install_zsh() -> anyhow::Result<InstallResult> {
    let completions_dir = data_home()?.join("zsh").join("site-functions");
    fs::create_dir_all(&completions_dir)?;
    let tg_path = completions_dir.join("_tg");
    write_file(&tg_path, ZSH_COMPLETION)?;

    let zshrc = home_dir()?.join(".zshrc");
    ensure_managed_block(
        &zshrc,
        ZSH_RC_MARKER_START,
        ZSH_RC_MARKER_END,
        &format!(
            "fpath=({path} $fpath)",
            path = shell_single_quote(&completions_dir)
        ),
    )?;

    Ok(InstallResult {
        shell: Shell::Zsh,
        written_files: vec![tg_path],
        updated_rc_files: vec![zshrc],
    })
}

fn install_fish() -> anyhow::Result<InstallResult> {
    let completions_dir = config_home()?.join("fish").join("completions");
    fs::create_dir_all(&completions_dir)?;
    let tg_path = completions_dir.join("tg.fish");
    let tracegrep_path = completions_dir.join("tracegrep.fish");
    write_file(&tg_path, FISH_COMPLETION)?;
    write_file(&tracegrep_path, FISH_COMPLETION)?;
    Ok(InstallResult {
        shell: Shell::Fish,
        written_files: vec![tg_path, tracegrep_path],
        updated_rc_files: Vec::new(),
    })
}

fn ensure_managed_block(
    rc_path: &Path,
    marker_start: &str,
    marker_end: &str,
    body: &str,
) -> anyhow::Result<()> {
    let existing = fs::read_to_string(rc_path).unwrap_or_default();
    if existing.contains(marker_start) && existing.contains(marker_end) {
        return Ok(());
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(marker_start);
    next.push('\n');
    next.push_str(body);
    next.push('\n');
    next.push_str(marker_end);
    next.push('\n');
    write_file(rc_path, &next)
}

fn write_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn shell_single_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', r#"'"'"'"#))
}

fn home_dir() -> anyhow::Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))
}

fn data_home() -> anyhow::Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(path));
    }
    Ok(home_dir()?.join(".local").join("share"))
}

fn config_home() -> anyhow::Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path));
    }
    Ok(home_dir()?.join(".config"))
}
