//! Mock implementation of [`ImagePusher`].

use minibox_core::domain::{AsAny, ImagePusher, PushProgress, PushResult, RegistryCredentials};
use async_trait::async_trait;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// MockImagePusher
// ---------------------------------------------------------------------------

/// State shared between the mock pusher and test handles.
struct MockImagePusherState {
    /// Tags that have been "pushed" — keyed by the full image ref string.
    pushed_tags: Vec<String>,
    /// Digest returned by the most recent push.
    last_digest: Option<String>,
}

/// In-memory mock for [`ImagePusher`].
///
/// Records all pushes; does not perform any network I/O.
/// The owning test can hold an `Arc<MockImagePusher>` and observe recorded
/// state via `has_tag` and `last_pushed_digest` after calls complete.
pub struct MockImagePusher {
    state: Mutex<MockImagePusherState>,
}

impl MockImagePusher {
    /// Create a new mock pusher with no recorded state.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MockImagePusherState {
                pushed_tags: vec![],
                last_digest: None,
            }),
        }
    }

    /// Returns `true` if `image_ref` has been pushed at least once.
    pub fn has_tag(&self, image_ref: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .pushed_tags
            .contains(&image_ref.to_string())
    }

    /// Returns the digest reported by the most recent push, or `None`.
    pub fn last_pushed_digest(&self) -> Option<String> {
        self.state.lock().unwrap().last_digest.clone()
    }
}

impl Default for MockImagePusher {
    fn default() -> Self {
        Self::new()
    }
}

impl AsAny for MockImagePusher {
    fn as_any(&self) -> &dyn ::std::any::Any {
        self
    }
}

#[async_trait]
impl ImagePusher for MockImagePusher {
    async fn push_image(
        &self,
        image_ref: &minibox_core::image::reference::ImageRef,
        _credentials: &RegistryCredentials,
        progress_tx: Option<tokio::sync::mpsc::Sender<PushProgress>>,
    ) -> anyhow::Result<PushResult> {
        let digest =
            "sha256:push000000000000000000000000000000000000000000000000000000000000".to_string();
        let ref_str = format!(
            "{}/{}/{}:{}",
            image_ref.registry, image_ref.namespace, image_ref.name, image_ref.tag
        );

        if let Some(tx) = progress_tx {
            let _ = tx
                .send(PushProgress {
                    layer_digest: digest.clone(),
                    bytes_uploaded: 1024,
                    total_bytes: 1024,
                })
                .await;
        }

        let mut state = self.state.lock().unwrap();
        state.pushed_tags.push(ref_str);
        state.last_digest = Some(digest.clone());

        Ok(PushResult {
            digest,
            size_bytes: 1024,
        })
    }
}
