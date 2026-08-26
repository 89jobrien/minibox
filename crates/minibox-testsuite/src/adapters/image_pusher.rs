//! Conformance tests for the [`ImagePusher`] trait contract.
//!
//! All tests use `MockImagePusher` — no network I/O is performed.

use minibox::testing::mocks::push::MockImagePusher;
use minibox_core::domain::{ImagePusher, RegistryCredentials};
use minibox_core::image::reference::ImageRef;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

fn alpine() -> ImageRef {
    ImageRef::parse("alpine:3.18").expect("parse alpine ref")
}

const fn anon() -> RegistryCredentials {
    RegistryCredentials::Anonymous
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

crate::conformance_test! {
    name: "push_image_returns_digest",
    adapter: "image_pusher",
    capability: PushToRegistry,
    category: Unit,
    |ctx| {
        let mock = MockImagePusher::new();
        let result = rt().block_on(mock.push_image(&alpine(), &anon(), None));
        if let Some(r) = ctx.assert_ok(result, "push_image should succeed") {
            ctx.assert_true(!r.digest.is_empty(), "digest must be non-empty");
        }
        ctx.result()
    }
}

crate::conformance_test! {
    name: "push_image_records_tag",
    adapter: "image_pusher",
    capability: PushToRegistry,
    category: Unit,
    |ctx| {
        let mock = MockImagePusher::new();
        rt().block_on(mock.push_image(&alpine(), &anon(), None))
            .expect("push");
        ctx.assert_true(
            mock.last_pushed_digest().is_some(),
            "last_pushed_digest should be set after push",
        );
        ctx.result()
    }
}

crate::conformance_test! {
    name: "push_image_failure_returns_err",
    adapter: "image_pusher",
    capability: PushToRegistry,
    category: EdgeCase,
    |ctx| {
        let mock = MockImagePusher::new().with_failure();
        let result = rt().block_on(mock.push_image(&alpine(), &anon(), None));
        ctx.assert_err(result, "push_image with failure configured must return Err");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "push_image_sends_progress",
    adapter: "image_pusher",
    capability: PushToRegistry,
    category: Unit,
    |ctx| {
        let mock = MockImagePusher::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let result =
            rt().block_on(mock.push_image(&alpine(), &anon(), Some(crate::progress::tokio_progress_sink(tx))));
        ctx.assert_ok(result, "push_image with progress channel should succeed");
        let got = rt().block_on(rx.recv());
        ctx.assert_true(got.is_some(), "at least one progress event should be sent");
        ctx.result()
    }
}
