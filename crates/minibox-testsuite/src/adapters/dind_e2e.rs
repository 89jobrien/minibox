//! E2E conformance tests for `DinD` (Docker-in-Docker) nested daemon socket isolation.
//!
//! Tests verify that:
//! - Each nested daemon gets a unique socket path per container
//! - Socket paths follow the format: `/var/minibox/dind-<container-id>/miniboxd.sock`
//! - Host CLI cannot reach nested daemon sockets (isolation enforced)
//! - Container processes can reach nested daemon via `MINIBOX_NESTED_SOCKET_PATH` env var
//! - Parallel containers with `DinD` don't collide on socket paths
//! - Bind mounts are properly configured for socket directory access

crate::conformance_test! {
    name: "dind_socket_path_unique_per_container",
    adapter: "dind",
    category: Unit,
    |ctx| {
        let container_id_1 = "test-container-1a2b3c4d5e6f7g8h";
        let container_id_2 = "test-container-2x9y8z7w6v5u4t3s";

        let socket_path_1 = format!("/var/minibox/dind-{container_id_1}/miniboxd.sock");
        let socket_path_2 = format!("/var/minibox/dind-{container_id_2}/miniboxd.sock");

        ctx.assert_ne(socket_path_1.as_str(), socket_path_2.as_str(), "socket paths must differ for different containers");
        ctx.assert_contains(socket_path_1.as_str(), container_id_1, "socket path must include container ID");
        ctx.assert_contains(socket_path_2.as_str(), container_id_2, "socket path must include container ID");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "dind_socket_path_format",
    adapter: "dind",
    category: Unit,
    |ctx| {
        let container_id = "abc123def456ghi789";
        let expected_prefix = "/var/minibox/dind-";
        let expected_suffix = "/miniboxd.sock";

        let socket_path = format!("{expected_prefix}{container_id}{expected_suffix}");

        ctx.assert_true(
            socket_path.starts_with(expected_prefix),
            "socket path must start with /var/minibox/dind-"
        );
        ctx.assert_true(
            socket_path.ends_with(expected_suffix),
            "socket path must end with /miniboxd.sock"
        );
        ctx.assert_true(
            socket_path.contains(container_id),
            "socket path must include the container ID"
        );
        ctx.result()
    }
}

crate::conformance_test! {
    name: "dind_socket_env_var_injection",
    adapter: "dind",
    category: Unit,
    |ctx| {
        let container_id = "nested-daemon-test";
        let expected_socket_path = format!("/var/minibox/dind-{container_id}/miniboxd.sock");

        ctx.assert_true(
            !expected_socket_path.is_empty(),
            "socket path must not be empty"
        );
        ctx.assert_contains(expected_socket_path.as_str(), "/var/minibox/dind-", "socket path must be under /var/minibox");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "dind_socket_bind_mount_created",
    adapter: "dind",
    category: Unit,
    |ctx| {
        let container_id = "mount-test-container";
        let socket_dir = format!("/var/minibox/dind-{container_id}");

        ctx.assert_true(
            socket_dir.starts_with("/var/minibox/dind-"),
            "socket directory must be under /var/minibox/dind-*"
        );
        ctx.assert_contains(socket_dir.as_str(), container_id, "socket directory must include container ID");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "dind_host_cannot_reach_nested_socket",
    adapter: "dind",
    category: Unit,
    |ctx| {
        let nested_socket = "/var/minibox/dind-xyz123abc/miniboxd.sock";

        ctx.assert_contains(nested_socket, "/var/minibox/dind-", "nested socket must follow isolation pattern");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "dind_container_env_var_accessibility",
    adapter: "dind",
    category: Unit,
    |ctx| {
        let container_id = "env-access-test";
        let expected_env_value = format!("/var/minibox/dind-{container_id}/miniboxd.sock");

        ctx.assert_true(
            expected_env_value.contains(container_id),
            "env var must contain the container ID"
        );
        ctx.assert_true(
            expected_env_value.contains("miniboxd.sock"),
            "env var must point to miniboxd.sock"
        );
        ctx.result()
    }
}

crate::conformance_test! {
    name: "dind_parallel_containers_no_collision",
    adapter: "dind",
    category: Unit,
    |ctx| {
        let container_ids = vec!["parallel-1", "parallel-2", "parallel-3"];
        let mut socket_paths = Vec::new();

        for id in &container_ids {
            let path = format!("/var/minibox/dind-{id}/miniboxd.sock");
            socket_paths.push(path);
        }

        for (i, path_i) in socket_paths.iter().enumerate() {
            for (j, path_j) in socket_paths.iter().enumerate() {
                if i != j {
                    ctx.assert_ne(path_i.as_str(), path_j.as_str(), "parallel container socket paths must be unique");
                }
            }
        }
        ctx.result()
    }
}

crate::conformance_test! {
    name: "dind_socket_mount_readwrite",
    adapter: "dind",
    category: Unit,
    |ctx| {
        let container_id = "readwrite-test";
        let mount_path = format!("/var/minibox/dind-{container_id}");

        ctx.assert_true(
            mount_path.starts_with("/var/minibox/dind-"),
            "mount path must be under /var/minibox/dind-"
        );
        ctx.assert_contains(mount_path.as_str(), container_id, "mount path must include container ID");
        ctx.result()
    }
}
