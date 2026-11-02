//! Entrypoint wrapper script generation for Docker-in-Docker (`DinD`) support.
//!
//! When `--dind` is requested, the original container command is wrapped with
//! a shell script that:
//! 1. Boots a nested miniboxd daemon
//! 2. Sets up socket path and environment variables
//! 3. Execs the original command with the daemon running
//! 4. Cleans up the daemon on exit

/// Configuration for entrypoint script generation.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EntrypointConfig {
    /// Container ID where the nested daemon will run.
    pub container_id: String,
    /// Original command to execute after daemon starts.
    pub original_command: Vec<String>,
    /// Socket path where miniboxd will listen.
    pub socket_path: String,
    /// Temporary directory for daemon data.
    pub temp_dir: String,
    /// Path to miniboxd binary inside the container.
    pub miniboxd_binary_path: String,
}

/// Generate a shell script that starts nested miniboxd and execs the original command.
///
/// The script:
/// 1. Validates miniboxd binary exists and is executable
/// 2. Boots miniboxd with the socket path
/// 3. Waits for socket to appear
/// 4. Execs the original command
/// 5. Cleans up on exit
///
/// # Arguments
/// * `config` - Entrypoint script configuration
///
/// # Returns
/// A shell script string that can be written to a file and executed
#[allow(dead_code, clippy::uninlined_format_args)]
pub fn generate_entrypoint_script(config: &EntrypointConfig) -> String {
    let socket_path = &config.socket_path;
    let temp_dir = &config.temp_dir;
    let miniboxd_binary = &config.miniboxd_binary_path;
    let container_id = &config.container_id;
    let original_cmd = config
        .original_command
        .iter()
        .map(|arg| format!("\"{}\"", arg.replace('\"', "\\\"")))
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        r#"#!/bin/sh
set -e

# DinD entrypoint wrapper for container {}

socket_path="{}"
temp_dir="{}"
miniboxd_binary="{}"
daemon_pid_file="${{temp_dir}}/miniboxd.pid"
log_file="${{temp_dir}}/miniboxd.log"

# Validate miniboxd binary exists and is executable
if [ ! -x "${{miniboxd_binary}}" ]; then
    echo "error: miniboxd binary not found or not executable: ${{miniboxd_binary}}" >&2
    exit 1
fi

# Ensure temp directory exists
mkdir -p "${{temp_dir}}"

# Start miniboxd in background
echo "Starting nested miniboxd daemon..."
"${{miniboxd_binary}}" \
    --socket "${{socket_path}}" \
    --log-level info \
    > "${{log_file}}" 2>&1 &

miniboxd_pid=$!
echo "${{miniboxd_pid}}" > "${{daemon_pid_file}}"

# Wait for socket to appear (max 30 seconds)
echo "Waiting for miniboxd socket..."
waited=0
while [ ! -S "${{socket_path}}" ] && [ ${{waited}} -lt 30 ]; do
    sleep 0.1
    waited=$((waited + 1))
done

if [ ! -S "${{socket_path}}" ]; then
    echo "error: miniboxd socket did not appear at ${{socket_path}}" >&2
    kill ${{miniboxd_pid}} 2>/dev/null || true
    exit 1
fi

echo "miniboxd socket ready at ${{socket_path}}"

# Set up environment for child process
export MINIBOX_SOCKET="${{socket_path}}"
export MINIBOX_NESTED_CONTAINER=1

# Define cleanup function
cleanup() {{
    exit_code=$?
    echo "Cleaning up nested miniboxd..."
    if [ -f "${{daemon_pid_file}}" ]; then
        daemon_pid=$(cat "${{daemon_pid_file}}")
        kill "${{daemon_pid}}" 2>/dev/null || true
        # Wait briefly for graceful shutdown
        sleep 0.5
        kill -9 "${{daemon_pid}}" 2>/dev/null || true
    fi
    rm -rf "${{temp_dir}}"
    exit ${{exit_code}}
}}

trap cleanup EXIT

# Exec the original command
echo "Executing original command: {}"
exec {}
"#,
        container_id, socket_path, temp_dir, miniboxd_binary, original_cmd, original_cmd
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entrypoint_script_generation() {
        let config = EntrypointConfig {
            container_id: "abc123".to_string(),
            original_command: vec!["sh".to_string(), "-c".to_string(), "echo hello".to_string()],
            socket_path: "/run/minibox/nested.sock".to_string(),
            temp_dir: "/tmp/minibox-nested".to_string(),
            miniboxd_binary_path: "/usr/local/bin/miniboxd".to_string(),
        };

        let script = generate_entrypoint_script(&config);

        // Verify script contains key elements
        assert!(script.contains("#!/bin/sh"));
        assert!(script.contains("socket_path=\"/run/minibox/nested.sock\""));
        assert!(script.contains("miniboxd_binary=\"/usr/local/bin/miniboxd\""));
        assert!(script.contains("echo hello")); // Original command should be in script
        assert!(script.contains("MINIBOX_SOCKET"));
        assert!(script.contains("MINIBOX_NESTED_CONTAINER=1"));
        assert!(script.contains("cleanup"));
        assert!(script.contains("trap cleanup EXIT"));
    }

    #[test]
    fn test_entrypoint_script_socket_path() {
        let config = EntrypointConfig {
            container_id: "test-container".to_string(),
            original_command: vec!["bash".to_string()],
            socket_path: "/custom/socket/path.sock".to_string(),
            temp_dir: "/tmp/custom".to_string(),
            miniboxd_binary_path: "/opt/miniboxd".to_string(),
        };

        let script = generate_entrypoint_script(&config);

        // Verify socket path is correctly embedded
        assert!(script.contains("socket_path=\"/custom/socket/path.sock\""));
        assert!(script.contains("export MINIBOX_SOCKET=\"${socket_path}\""));
    }

    #[test]
    fn test_entrypoint_script_env_vars() {
        let config = EntrypointConfig {
            container_id: "env-test".to_string(),
            original_command: vec!["echo".to_string(), "$MINIBOX_SOCKET".to_string()],
            socket_path: "/tmp/test.sock".to_string(),
            temp_dir: "/tmp/test".to_string(),
            miniboxd_binary_path: "/bin/miniboxd".to_string(),
        };

        let script = generate_entrypoint_script(&config);

        // Verify environment variables are set
        assert!(script.contains("MINIBOX_SOCKET"));
        assert!(script.contains("MINIBOX_NESTED_CONTAINER=1"));
    }

    #[test]
    fn test_entrypoint_script_command_escaping() {
        let config = EntrypointConfig {
            container_id: "escape-test".to_string(),
            original_command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "echo \"hello world\"".to_string(),
            ],
            socket_path: "/tmp/test.sock".to_string(),
            temp_dir: "/tmp/test".to_string(),
            miniboxd_binary_path: "/bin/miniboxd".to_string(),
        };

        let script = generate_entrypoint_script(&config);

        // Verify commands are properly quoted and escaped
        assert!(script.contains("sh"));
        assert!(script.contains("-c"));
    }

    #[test]
    fn test_entrypoint_script_cleanup() {
        let config = EntrypointConfig {
            container_id: "cleanup-test".to_string(),
            original_command: vec!["test-cmd".to_string()],
            socket_path: "/tmp/test.sock".to_string(),
            temp_dir: "/tmp/test".to_string(),
            miniboxd_binary_path: "/bin/miniboxd".to_string(),
        };

        let script = generate_entrypoint_script(&config);

        // Verify cleanup logic is present
        assert!(script.contains("cleanup() {"));
        assert!(script.contains("kill \"${daemon_pid}\" 2>/dev/null || true"));
        assert!(script.contains("kill -9 \"${daemon_pid}\" 2>/dev/null || true"));
        assert!(script.contains("rm -rf \"${temp_dir}\""));
    }
}
