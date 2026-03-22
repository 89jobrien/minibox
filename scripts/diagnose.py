#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "claude-agent-sdk",
# ]
# ///
"""Diagnose minibox container failures from daemon logs and cgroup/mount state."""

import argparse
import asyncio

from claude_agent_sdk import ClaudeAgentOptions, query


async def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--container", help="Container ID to focus on (optional)")
    parser.add_argument(
        "--lines", type=int, default=200, help="Daemon log lines to inspect (default: 200)"
    )
    args = parser.parse_args()

    container_hint = (
        f"Focus on container ID: {args.container}" if args.container else "Focus on the most recent failure."
    )

    prompt = f"""Diagnose a minibox container failure. {container_hint}

Gather evidence in this order:
1. Run `journalctl -u miniboxd -n {args.lines} --no-pager` to get recent daemon logs
   (if that fails, try `journalctl -n {args.lines} --no-pager | grep -i minibox`)
2. Run `mount | grep minibox` to check for leaked overlay mounts
3. Check cgroup state: `ls /sys/fs/cgroup/minibox.slice/miniboxd.service/ 2>/dev/null || echo 'no minibox cgroups'`
4. If a container ID is known, read:
   - `/run/minibox/containers/<id>/` for runtime state
   - `/sys/fs/cgroup/minibox.slice/miniboxd.service/<id>/` for resource limits
5. Check for common failure modes:
   - `pivot_root` EINVAL → MS_PRIVATE not set before mount namespace ops
   - overlay ENOTDIR / EINVAL → malformed lowerdir paths
   - cgroup EACCES / ENOENT → cgroup hierarchy missing or wrong path
   - clone EPERM → missing CAP_SYS_ADMIN (check if MINIBOX_ADAPTER=gke needed)
   - exec ENOENT → image layer extraction failed or wrong rootfs path

Report:
- **Root cause**: specific syscall/error and why it failed
- **Evidence**: the exact log lines or file contents that confirm it
- **Fix**: minimal change (env var, config, or code pointer from CLAUDE.md)
- **Confidence**: high / medium / low"""

    print("Diagnosing minibox failure...\n")

    async for message in query(
        prompt=prompt,
        options=ClaudeAgentOptions(
            allowed_tools=["Bash", "Read", "Glob"],
            permission_mode="acceptEdits",
        ),
    ):
        if hasattr(message, "result"):
            print(message.result)


if __name__ == "__main__":
    asyncio.run(main())
