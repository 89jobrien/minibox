//! Basic Dockerfile parser.
//!
//! Supports the instruction subset needed for ~90% of real Dockerfiles:
//! FROM, RUN, COPY, ADD, ENV, ARG, WORKDIR, CMD, ENTRYPOINT, EXPOSE, LABEL, USER.
//!
//! Does NOT support: HEALTHCHECK, VOLUME, ONBUILD, SHELL, STOPSIGNAL,
//! `BuildKit` --mount syntax, multi-stage (only first FROM is used).

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
    },
    /// Execute a command while building the image.
    Run(ShellOrExec),
    /// Copy local context paths into the image.
    Copy {
        /// Source paths relative to the build context.
        srcs: Vec<PathBuf>,
        /// Destination path in the image.
        dest: PathBuf,
    },
    /// Add local paths or URLs into the image.
    Add {
        /// Local or remote sources.
        srcs: Vec<AddSource>,
        /// Destination path in the image.
        dest: PathBuf,
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

    for (line_num, line) in lines.iter().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(comment) = line.strip_prefix('#') {
            instructions.push(Instruction::Comment(comment.trim().to_string()));
            continue;
        }

        let (keyword, rest) = split_keyword(line);
        let keyword_upper = keyword.to_uppercase();

        if keyword_upper != "FROM" && !found_from {
            bail!(
                "line {}: first instruction must be FROM, got {}",
                line_num + 1,
                keyword_upper
            );
        }

        let instr = match keyword_upper.as_str() {
            "FROM" => {
                found_from = true;
                parse_from(rest)?
            }
            "RUN" => Instruction::Run(parse_shell_or_exec(rest)?),
            "CMD" => Instruction::Cmd(parse_shell_or_exec(rest)?),
            "ENTRYPOINT" => Instruction::Entrypoint(parse_shell_or_exec(rest)?),
            "COPY" => parse_copy(rest)?,
            "ADD" => parse_add(rest)?,
            "ENV" => Instruction::Env(parse_env(rest)),
            "ARG" => parse_arg(rest),
            "WORKDIR" => Instruction::Workdir(PathBuf::from(rest)),
            "EXPOSE" => parse_expose(rest)?,
            "LABEL" => Instruction::Label(parse_env(rest)),
            "USER" => parse_user(rest),
            other => bail!("line {}: unsupported instruction: {}", line_num + 1, other),
        };

        instructions.push(instr);
    }

    if !found_from {
        bail!("Dockerfile has no FROM instruction");
    }

    Ok(instructions)
}

fn join_continuations(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    for line in input.lines() {
        if let Some(stripped) = line.strip_suffix('\\') {
            current.push_str(stripped);
            current.push(' ');
        } else {
            current.push_str(line);
            result.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

fn split_keyword(line: &str) -> (&str, &str) {
    line.find(char::is_whitespace)
        .map_or((line, ""), |pos| (&line[..pos], line[pos..].trim()))
}

fn parse_shell_or_exec(s: &str) -> Result<ShellOrExec> {
    let s = s.trim();
    if s.starts_with('[') {
        let args: Vec<String> =
            serde_json::from_str(s).with_context(|| format!("invalid exec form JSON: {s}"))?;
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
    let (image_tag, alias) = if parts.len() >= 3 && parts[1].to_uppercase() == "AS" {
        (parts[0], Some(parts[2].to_string()))
    } else {
        (parts[0], None)
    };

    let (image, tag) = if let Some((img, tag)) = image_tag.rsplit_once(':') {
        (img.to_string(), tag.to_string())
    } else {
        (image_tag.to_string(), "latest".to_string())
    };

    Ok(Instruction::From { image, tag, alias })
}

fn parse_copy(s: &str) -> Result<Instruction> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 {
        bail!("COPY requires at least one source and a destination");
    }
    let dest = PathBuf::from(parts[parts.len() - 1]);
    let srcs = parts[..parts.len() - 1].iter().map(PathBuf::from).collect();
    Ok(Instruction::Copy { srcs, dest })
}

fn parse_add(s: &str) -> Result<Instruction> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 {
        bail!("ADD requires at least one source and a destination");
    }
    let dest = PathBuf::from(parts[parts.len() - 1]);
    let srcs = parts[..parts.len() - 1]
        .iter()
        .map(|p| {
            let s = (*p).to_string();
            if s.starts_with("http://") || s.starts_with("https://") {
                AddSource::Url(s)
            } else {
                AddSource::Local(PathBuf::from(s))
            }
        })
        .collect();
    Ok(Instruction::Add { srcs, dest })
}

fn parse_env(s: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    if s.contains('=') {
        // KEY=VALUE form (possibly multiple pairs)
        for part in s.split_whitespace() {
            if let Some((k, v)) = part.split_once('=') {
                pairs.push((k.to_string(), v.to_string()));
            }
        }
    } else {
        // Legacy: ENV KEY VALUE
        if let Some((k, v)) = s.split_once(char::is_whitespace) {
            pairs.push((k.to_string(), v.trim().to_string()));
        }
    }
    pairs
}

fn parse_arg(s: &str) -> Instruction {
    if let Some((name, default)) = s.split_once('=') {
        Instruction::Arg {
            name: name.trim().to_string(),
            default: Some(default.trim().to_string()),
        }
    } else {
        Instruction::Arg {
            name: s.trim().to_string(),
            default: None,
        }
    }
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

fn parse_user(s: &str) -> Instruction {
    if let Some((name, group)) = s.split_once(':') {
        Instruction::User {
            name: name.to_string(),
            group: Some(group.to_string()),
        }
    } else {
        Instruction::User {
            name: s.to_string(),
            group: None,
        }
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
}
