//! Dockerfile parser for the native minibox image builder.
//!
//! The public AST preserves stage aliases, COPY sources and flags, command
//! forms, and image metadata instructions. Execution support is intentionally
//! separate from parsing so callers can validate a build before mutating the
//! image store.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
/// A command expressed in shell form or JSON exec form.
pub enum ShellOrExec {
    /// Shell-form command executed through the default shell.
    Shell(String),
    /// Exec-form command represented as an argument vector.
    Exec(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Source accepted by an `ADD` instruction.
pub enum AddSource {
    /// File or directory from the local build context.
    Local(PathBuf),
    /// Remote URL parsed from the instruction for downstream handling.
    Url(String),
}

/// Reference to a previously declared build stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageReference {
    /// Case-insensitive stage alias declared by `FROM ... AS name`.
    Alias(String),
    /// Zero-based index of a previously declared stage.
    Index(usize),
}

/// Resolved source of a `FROM` instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromSource {
    /// External image reference.
    Image,
    /// Previously declared stage reference.
    Stage(StageReference),
}

impl StageReference {
    fn parse(value: &str) -> Self {
        value
            .parse::<usize>()
            .map_or_else(|_| Self::Alias(value.to_string()), Self::Index)
    }
}

/// User and optional group requested by `COPY --chown`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyOwnership {
    /// User name or numeric user ID.
    pub user: String,
    /// Optional group name or numeric group ID.
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Parsed Dockerfile instruction recognized by the parser.
pub enum Instruction {
    /// Select a base image and optional stage alias.
    From {
        /// Base image name.
        image: String,
        /// Base image tag.
        tag: String,
        /// Optional build-stage alias.
        alias: Option<String>,
        /// Whether this starts from an external image or prior stage.
        source: FromSource,
    },
    /// Execute a command while building the image.
    Run(ShellOrExec),
    /// Copy local context paths into the image.
    Copy {
        /// Source paths relative to the build context.
        srcs: Vec<PathBuf>,
        /// Destination path in the image.
        dest: PathBuf,
        /// Optional prior stage supplying the source paths.
        from: Option<StageReference>,
        /// Optional ownership applied to copied entries.
        chown: Option<CopyOwnership>,
        /// Optional octal mode applied to copied files and directories.
        chmod: Option<u32>,
    },
    /// Add local paths or URLs into the image.
    Add {
        /// Local or remote sources.
        srcs: Vec<AddSource>,
        /// Destination path in the image.
        dest: PathBuf,
        /// Optional ownership applied to local sources.
        chown: Option<CopyOwnership>,
        /// Optional octal mode applied to local sources.
        chmod: Option<u32>,
    },
    /// Set image environment variables.
    Env(Vec<(String, String)>),
    /// Declare a build argument and optional default value.
    Arg {
        /// Build argument name.
        name: String,
        /// Optional default value.
        default: Option<String>,
    },
    /// Set the working directory for subsequent instructions.
    Workdir(PathBuf),
    /// Set the image's default command.
    Cmd(ShellOrExec),
    /// Set the image's entrypoint.
    Entrypoint(ShellOrExec),
    /// Document a port exposed by the image.
    Expose {
        /// Port number.
        port: u16,
        /// Transport protocol, usually `tcp` or `udp`.
        proto: String,
    },
    /// Set image labels.
    Label(Vec<(String, String)>),
    /// Set the user and optional group for subsequent commands.
    User {
        /// User name or numeric identifier.
        name: String,
        /// Optional group name or numeric identifier.
        group: Option<String>,
    },
    /// Declare mount points persisted in the image config.
    Volume(Vec<PathBuf>),
    /// Preserve a source comment.
    Comment(String),
}

/// # Errors
///
/// Returns an error if the Dockerfile contains unknown or malformed instructions.
pub fn parse(input: &str) -> Result<Vec<Instruction>> {
    let lines = join_continuations(input);
    let mut instructions = Vec::new();
    let mut found_from = false;
    let mut stage_aliases = Vec::<String>::new();

    for logical_line in &lines {
        let line = logical_line.text.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(comment) = line.strip_prefix('#') {
            instructions.push(Instruction::Comment(comment.trim().to_string()));
            continue;
        }

        let (keyword, rest) = split_keyword(line);
        let keyword_upper = keyword.to_uppercase();

        if keyword_upper != "FROM" && keyword_upper != "ARG" && !found_from {
            bail!(
                "line {}: first instruction must be FROM, got {}",
                logical_line.number,
                keyword_upper
            );
        }

        let line_context = || format!("line {}: {line}", logical_line.number);
        let instr = match keyword_upper.as_str() {
            "FROM" => {
                found_from = true;
                let mut instruction = parse_from(rest).with_context(line_context)?;
                if let Instruction::From {
                    image,
                    tag,
                    alias,
                    source,
                } = &mut instruction
                {
                    *source = if tag == "latest" {
                        if let Ok(index) = image.parse::<usize>() {
                            FromSource::Stage(StageReference::Index(index))
                        } else if stage_aliases
                            .iter()
                            .any(|candidate| candidate.eq_ignore_ascii_case(image))
                        {
                            FromSource::Stage(StageReference::Alias(image.clone()))
                        } else {
                            FromSource::Image
                        }
                    } else {
                        FromSource::Image
                    };
                    if let Some(alias) = alias {
                        stage_aliases.push(alias.clone());
                    }
                }
                instruction
            }
            "RUN" => Instruction::Run(parse_shell_or_exec(rest).with_context(line_context)?),
            "CMD" => Instruction::Cmd(parse_shell_or_exec(rest).with_context(line_context)?),
            "ENTRYPOINT" => {
                Instruction::Entrypoint(parse_shell_or_exec(rest).with_context(line_context)?)
            }
            "COPY" => parse_copy(rest).with_context(line_context)?,
            "ADD" => parse_add(rest).with_context(line_context)?,
            "ENV" => Instruction::Env(
                parse_key_value_pairs(rest, true, true).with_context(line_context)?,
            ),
            "ARG" => parse_arg(rest).with_context(line_context)?,
            "WORKDIR" => {
                if rest.is_empty() {
                    bail!("line {}: WORKDIR requires a path", logical_line.number);
                }
                Instruction::Workdir(PathBuf::from(rest))
            }
            "EXPOSE" => parse_expose(rest).with_context(line_context)?,
            "LABEL" => Instruction::Label(
                parse_key_value_pairs(rest, false, false).with_context(line_context)?,
            ),
            "USER" => parse_user(rest).with_context(line_context)?,
            "VOLUME" => parse_volume(rest).with_context(line_context)?,
            other => bail!(
                "line {}: unsupported instruction: {}",
                logical_line.number,
                other
            ),
        };

        instructions.push(instr);
    }

    if !found_from {
        bail!("Dockerfile has no FROM instruction");
    }

    Ok(instructions)
}

struct LogicalLine {
    number: usize,
    text: String,
}

fn join_continuations(input: &str) -> Vec<LogicalLine> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut start_line = 1;
    for (index, line) in input.lines().enumerate() {
        if current.is_empty() {
            start_line = index + 1;
        }
        if let Some(stripped) = line.strip_suffix('\\') {
            current.push_str(stripped);
            current.push(' ');
        } else {
            current.push_str(line);
            result.push(LogicalLine {
                number: start_line,
                text: std::mem::take(&mut current),
            });
        }
    }
    if !current.is_empty() {
        result.push(LogicalLine {
            number: start_line,
            text: current,
        });
    }
    result
}

fn split_keyword(line: &str) -> (&str, &str) {
    line.find(char::is_whitespace)
        .map_or((line, ""), |pos| (&line[..pos], line[pos..].trim()))
}

fn parse_shell_or_exec(s: &str) -> Result<ShellOrExec> {
    let s = s.trim();
    if s.is_empty() {
        bail!("command must not be empty");
    }
    if s.starts_with('[') {
        let args: Vec<String> =
            serde_json::from_str(s).with_context(|| format!("invalid exec form JSON: {s}"))?;
        if args.is_empty() || args.iter().any(String::is_empty) {
            bail!("exec form requires non-empty arguments");
        }
        Ok(ShellOrExec::Exec(args))
    } else {
        Ok(ShellOrExec::Shell(s.to_string()))
    }
}

fn parse_from(s: &str) -> Result<Instruction> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.is_empty() {
        bail!("FROM requires an image argument");
    }
    let (image_tag, alias) = match parts.as_slice() {
        [image] => (*image, None),
        [image, as_keyword, alias] if as_keyword.eq_ignore_ascii_case("AS") => {
            (*image, Some((*alias).to_string()))
        }
        _ => bail!("FROM expects IMAGE or IMAGE AS ALIAS"),
    };

    let last_slash = image_tag.rfind('/');
    let tag_separator = image_tag.rfind(':').filter(|separator| {
        last_slash.is_none_or(|slash| *separator > slash) && !image_tag.contains('@')
    });
    let (image, tag) = if let Some(separator) = tag_separator {
        (
            image_tag[..separator].to_string(),
            image_tag[separator + 1..].to_string(),
        )
    } else {
        (image_tag.to_string(), "latest".to_string())
    };
    if image.is_empty() || tag.is_empty() {
        bail!("FROM image and tag must not be empty");
    }

    Ok(Instruction::From {
        image,
        tag,
        alias,
        source: FromSource::Image,
    })
}

fn parse_copy(s: &str) -> Result<Instruction> {
    let (flags, payload) = parse_copy_flags(s, true)?;
    let paths = parse_path_list(payload, "COPY")?;
    let (srcs, dest) = split_sources_and_destination(paths, "COPY")?;
    Ok(Instruction::Copy {
        srcs,
        dest,
        from: flags.from,
        chown: flags.chown,
        chmod: flags.chmod,
    })
}

fn parse_add(s: &str) -> Result<Instruction> {
    let (flags, payload) = parse_copy_flags(s, false)?;
    let paths = parse_path_list(payload, "ADD")?;
    let (paths, dest) = split_sources_and_destination(paths, "ADD")?;
    let srcs = paths
        .into_iter()
        .map(|path| {
            let s = path.to_string_lossy().into_owned();
            if s.starts_with("http://") || s.starts_with("https://") {
                AddSource::Url(s)
            } else {
                AddSource::Local(path)
            }
        })
        .collect();
    Ok(Instruction::Add {
        srcs,
        dest,
        chown: flags.chown,
        chmod: flags.chmod,
    })
}

#[derive(Default)]
struct CopyFlags {
    from: Option<StageReference>,
    chown: Option<CopyOwnership>,
    chmod: Option<u32>,
}

fn parse_copy_flags(mut input: &str, allow_from: bool) -> Result<(CopyFlags, &str)> {
    let mut flags = CopyFlags::default();
    loop {
        input = input.trim_start();
        if !input.starts_with("--") {
            return Ok((flags, input));
        }

        let (token, remaining) = take_word(input)?;
        let (name, inline_value) = token
            .strip_prefix("--")
            .and_then(|flag| flag.split_once('='))
            .map_or_else(
                || (token.trim_start_matches("--"), None),
                |(name, value)| (name, Some(value)),
            );
        let (value, next) = if let Some(value) = inline_value {
            (value.to_string(), remaining)
        } else {
            let (value, next) = take_word(remaining.trim_start())?;
            (value, next)
        };

        match name {
            "from" if allow_from => flags.from = Some(StageReference::parse(&value)),
            "from" => bail!("ADD does not support --from"),
            "chown" => flags.chown = Some(parse_copy_ownership(&value)?),
            "chmod" => flags.chmod = Some(parse_octal_mode(&value)?),
            other => bail!("unsupported copy flag --{other}"),
        }
        input = next;
    }
}

fn parse_copy_ownership(value: &str) -> Result<CopyOwnership> {
    let (user, group) = value
        .split_once(':')
        .map_or((value, None), |(user, group)| (user, Some(group)));
    if user.is_empty() || group.is_some_and(str::is_empty) {
        bail!("--chown requires USER or USER:GROUP");
    }
    Ok(CopyOwnership {
        user: user.to_string(),
        group: group.map(str::to_string),
    })
}

fn parse_octal_mode(value: &str) -> Result<u32> {
    let digits = value.strip_prefix("0o").unwrap_or(value);
    let mode = u32::from_str_radix(digits, 8)
        .with_context(|| format!("invalid --chmod octal mode: {value}"))?;
    if mode > 0o777 {
        bail!("invalid --chmod mode outside 000-777: {value}");
    }
    Ok(mode)
}

fn parse_path_list(input: &str, instruction: &str) -> Result<Vec<PathBuf>> {
    let values = if input.trim_start().starts_with('[') {
        serde_json::from_str::<Vec<String>>(input)
            .with_context(|| format!("invalid {instruction} JSON array"))?
    } else {
        split_shell_words(input)?
    };
    Ok(values.into_iter().map(PathBuf::from).collect())
}

fn split_sources_and_destination(
    mut paths: Vec<PathBuf>,
    instruction: &str,
) -> Result<(Vec<PathBuf>, PathBuf)> {
    if paths.len() < 2 {
        bail!("{instruction} requires at least one source and a destination");
    }
    let dest = paths
        .pop()
        .context("source list unexpectedly missing destination")?;
    Ok((paths, dest))
}

fn take_word(input: &str) -> Result<(String, &str)> {
    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    if end == 0 {
        bail!("expected flag value");
    }
    Ok((input[..end].to_string(), &input[end..]))
}

fn split_shell_words(input: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in input.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            } else {
                current.push(character);
            }
            continue;
        }
        if character.is_whitespace() && quote.is_none() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if escaped || quote.is_some() {
        bail!("unterminated escape or quote");
    }
    if !current.is_empty() {
        words.push(current);
    }
    Ok(words)
}

fn parse_volume(input: &str) -> Result<Instruction> {
    let paths = parse_path_list(input, "VOLUME")?;
    if paths.is_empty() {
        bail!("VOLUME requires at least one path");
    }
    Ok(Instruction::Volume(paths))
}

fn parse_key_value_pairs(
    s: &str,
    allow_legacy: bool,
    validate_env_name: bool,
) -> Result<Vec<(String, String)>> {
    let words = split_shell_words(s)?;
    let Some(first) = words.first() else {
        bail!("instruction requires at least one key/value pair");
    };
    if first.contains('=') {
        words
            .into_iter()
            .map(|word| {
                let (key, value) = word
                    .split_once('=')
                    .context("all key/value entries must use KEY=VALUE form")?;
                if key.is_empty() {
                    bail!("key must not be empty");
                }
                if validate_env_name {
                    validate_variable_name(key)?;
                }
                Ok((key.to_string(), value.to_string()))
            })
            .collect()
    } else if allow_legacy && words.len() >= 2 {
        if validate_env_name {
            validate_variable_name(first)?;
        }
        Ok(vec![(first.clone(), words[1..].join(" "))])
    } else {
        bail!("instruction requires KEY=VALUE form")
    }
}

fn parse_arg(s: &str) -> Result<Instruction> {
    if let Some((name, default)) = s.split_once('=') {
        let name = name.trim();
        validate_variable_name(name)?;
        Ok(Instruction::Arg {
            name: name.trim().to_string(),
            default: Some(default.trim().to_string()),
        })
    } else {
        let name = s.trim();
        validate_variable_name(name)?;
        Ok(Instruction::Arg {
            name: name.to_string(),
            default: None,
        })
    }
}

fn validate_variable_name(name: &str) -> Result<()> {
    let mut characters = name.chars();
    let valid_start = characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_');
    if !valid_start
        || !characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        bail!("invalid variable name: {name:?}");
    }
    Ok(())
}

fn parse_expose(s: &str) -> Result<Instruction> {
    let (port_str, proto) = if let Some((p, proto)) = s.split_once('/') {
        (p, proto.to_string())
    } else {
        (s.trim(), "tcp".to_string())
    };
    let port = port_str
        .trim()
        .parse::<u16>()
        .with_context(|| format!("invalid port: {port_str}"))?;
    Ok(Instruction::Expose { port, proto })
}

fn parse_user(s: &str) -> Result<Instruction> {
    if s.trim().is_empty() {
        bail!("USER requires a user name or numeric ID");
    }
    if let Some((name, group)) = s.split_once(':') {
        if name.is_empty() || group.is_empty() {
            bail!("USER requires USER or USER:GROUP");
        }
        Ok(Instruction::User {
            name: name.to_string(),
            group: Some(group.to_string()),
        })
    } else {
        Ok(Instruction::User {
            name: s.to_string(),
            group: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_from_with_tag() {
        let instrs = parse("FROM alpine:3.18\n").expect("valid dockerfile");
        assert!(matches!(
            &instrs[0],
            Instruction::From { image, tag, .. } if image == "alpine" && tag == "3.18"
        ));
    }

    #[test]
    fn parse_from_no_tag_defaults_latest() {
        let instrs = parse("FROM alpine\n").expect("valid dockerfile");
        assert!(matches!(
            &instrs[0],
            Instruction::From { tag, .. } if tag == "latest"
        ));
    }

    #[test]
    fn parse_run_shell_form() {
        let instrs = parse("FROM alpine\nRUN echo hello\n").expect("valid dockerfile");
        assert!(matches!(&instrs[1], Instruction::Run(ShellOrExec::Shell(s)) if s == "echo hello"));
    }

    #[test]
    fn parse_run_exec_form() {
        let instrs = parse("FROM alpine\nRUN [\"echo\", \"hello\"]\n").expect("valid dockerfile");
        assert!(matches!(
            &instrs[1],
            Instruction::Run(ShellOrExec::Exec(args)) if args[0] == "echo"
        ));
    }

    #[test]
    fn parse_copy() {
        let instrs = parse("FROM alpine\nCOPY src/ /app/\n").expect("valid dockerfile");
        assert!(
            matches!(&instrs[1], Instruction::Copy { dest, .. } if dest.to_string_lossy() == "/app/")
        );
    }

    #[test]
    fn parse_env_equals_form() {
        let instrs = parse("FROM alpine\nENV FOO=bar BAZ=qux\n").expect("valid dockerfile");
        assert!(
            matches!(&instrs[1], Instruction::Env(pairs) if pairs[0] == ("FOO".to_string(), "bar".to_string()))
        );
    }

    #[test]
    fn parse_comment_skipped_but_from_present() {
        let instrs = parse("# comment\nFROM alpine\n").expect("valid dockerfile");
        assert!(instrs.iter().any(|i| matches!(i, Instruction::From { .. })));
    }

    #[test]
    fn parse_error_no_from() {
        let result = parse("RUN echo hello\n");
        assert!(result.is_err());
    }

    #[test]
    fn parse_workdir() {
        let instrs = parse("FROM alpine\nWORKDIR /app\n").expect("valid dockerfile");
        assert!(matches!(&instrs[1], Instruction::Workdir(p) if p.to_string_lossy() == "/app"));
    }

    #[test]
    fn parse_arg_with_default() {
        let instrs = parse("FROM alpine\nARG VERSION=1.0\n").expect("valid dockerfile");
        assert!(
            matches!(&instrs[1], Instruction::Arg { name, default } if name == "VERSION" && default.as_deref() == Some("1.0"))
        );
    }

    #[test]
    fn parse_multiple_stages_and_copy_flags() {
        let dockerfile = "FROM alpine AS build\nCOPY --from=build --chown 1000:1001 --chmod=0755 /bin/tool /usr/bin/tool\nFROM 0 AS final\n";
        let instructions = parse(dockerfile).expect("multi-stage Dockerfile should parse");
        assert!(matches!(
            &instructions[1],
            Instruction::Copy {
                from: Some(StageReference::Alias(stage)),
                chown: Some(CopyOwnership { user, group: Some(group) }),
                chmod: Some(0o755),
                ..
            } if stage == "build" && user == "1000" && group == "1001"
        ));
        assert!(matches!(
            &instructions[2],
            Instruction::From {
                image,
                alias: Some(alias),
                source: FromSource::Stage(StageReference::Index(0)),
                ..
            } if image == "0" && alias == "final"
        ));
    }

    #[test]
    fn parse_copy_accepts_standard_and_equals_flag_forms() {
        let instructions = parse(
            "FROM scratch AS source\nCOPY --from source --chown=12:34 --chmod 0640 file /file\n",
        )
        .expect("COPY flag forms should parse");
        assert!(matches!(
            &instructions[1],
            Instruction::Copy {
                from: Some(StageReference::Alias(alias)),
                chown: Some(CopyOwnership { user, group: Some(group) }),
                chmod: Some(0o640),
                ..
            } if alias == "source" && user == "12" && group == "34"
        ));
    }

    #[test]
    fn parse_copy_json_array_form() {
        let dockerfile = "FROM scratch\nCOPY [\"file one\", \"file two\", \"/app/\"]\n";
        let instructions = parse(dockerfile).expect("JSON COPY should parse");
        assert!(matches!(
            &instructions[1],
            Instruction::Copy { srcs, dest, .. }
                if srcs == &[PathBuf::from("file one"), PathBuf::from("file two")]
                    && dest == &PathBuf::from("/app/")
        ));
    }

    #[test]
    fn parse_volume_shell_and_json_forms() {
        for dockerfile in [
            "FROM scratch\nVOLUME /data /cache\n",
            "FROM scratch\nVOLUME [\"/data\", \"/cache\"]\n",
        ] {
            let instructions = parse(dockerfile).expect("VOLUME should parse");
            assert!(matches!(
                &instructions[1],
                Instruction::Volume(paths)
                    if paths == &[PathBuf::from("/data"), PathBuf::from("/cache")]
            ));
        }
    }

    #[test]
    fn parse_error_includes_logical_line_context() {
        let error = parse("FROM scratch\nCOPY --chmod=invalid file /app/\n")
            .expect_err("invalid mode must fail");
        assert!(error.to_string().contains("line 2"), "error: {error:#}");
    }

    #[test]
    fn parse_quoted_env_and_label_values() {
        let instructions = parse(
            "FROM scratch\nENV GREETING=\"hello world\" OTHER='two words'\nLABEL org.example.note=\"quoted value\"\n",
        )
        .expect("quoted metadata should parse");
        assert!(matches!(
            &instructions[1],
            Instruction::Env(values)
                if values == &vec![
                    ("GREETING".to_string(), "hello world".to_string()),
                    ("OTHER".to_string(), "two words".to_string())
                ]
        ));
        assert!(matches!(
            &instructions[2],
            Instruction::Label(values)
                if values == &vec![("org.example.note".to_string(), "quoted value".to_string())]
        ));
    }

    #[test]
    fn parse_rejects_malformed_metadata_and_special_chmod_bits() {
        for dockerfile in [
            "FROM scratch\nENV KEY\n",
            "FROM scratch\nLABEL missing_equals\n",
            "FROM scratch\nCOPY --chmod=1755 file /file\n",
            "FROM scratch\nUSER \n",
        ] {
            let error = parse(dockerfile).expect_err("malformed instruction must fail");
            assert!(error.to_string().contains("line 2"), "error: {error:#}");
        }
    }

    #[test]
    fn parse_allows_global_arg_before_from() {
        let instructions =
            parse("ARG BASE=alpine\nFROM ${BASE}:3.21\n").expect("global ARG should parse");
        assert!(matches!(&instructions[0], Instruction::Arg { name, .. } if name == "BASE"));
        assert!(
            matches!(&instructions[1], Instruction::From { image, tag, .. } if image == "${BASE}" && tag == "3.21")
        );
    }
}
