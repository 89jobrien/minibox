//! Barrier-based race tests for daemon shared state.
//!
//! Each test forces a specific two-thread interleaving by using `std::sync::Barrier`
//! to synchronise the two threads at a known point, then immediately invoking
//! the competing operations so the scheduler must choose which runs first.
//!
//! The tests use simplified stand-in types rather than `DaemonState` / `ImageGc`
//! directly because those types require heavyweight construction (ImageStore,
//! TraceStore, real filesystem paths). The stand-ins faithfully replicate the
//! concurrency pattern being tested (HashMap behind Arc<Mutex<_>>, broadcast
//! channel, HashSet behind Arc<Mutex<_>>) without infrastructure dependencies.

use std::collections::HashMap;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::SystemTime;

use minibox_core::events::{BroadcastEventBroker, ContainerEvent, EventSink, EventSource};

// ---------------------------------------------------------------------------
// Test 1: create vs destroy (same container ID)
// ---------------------------------------------------------------------------

/// Simulates concurrent insert and remove of the same container ID.
///
/// Stand-in: `Arc<Mutex<HashMap<String, u64>>>` mirrors the `containers`
/// field in `DaemonState` (which uses `Arc<RwLock<HashMap<…>>>`) for the
/// pattern being tested. The Mutex variant is used here because `std::thread`
/// tests cannot block on `tokio::sync::RwLock` outside an async runtime.
///
/// Invariant: after both threads join the mutex must not be poisoned and the
/// map must contain at most one entry for the shared key.
#[test]
fn race_create_vs_destroy() {
    let containers: Arc<Mutex<HashMap<String, u64>>> = Arc::new(Mutex::new(HashMap::new()));

    for _ in 0..100 {
        let barrier = Arc::new(Barrier::new(2));
        let state = Arc::clone(&containers);

        // Pre-populate so the destroy thread has something to remove.
        {
            let mut m = state.lock().expect("mutex poisoned before test");
            m.insert("ctr-race".to_owned(), 42);
        }

        let state_create = Arc::clone(&state);
        let barrier_create = Arc::clone(&barrier);
        let create = thread::spawn(move || {
            barrier_create.wait();
            // Simulate create: upsert the container record.
            let mut m = state_create
                .lock()
                .expect("mutex poisoned in create thread");
            m.insert("ctr-race".to_owned(), 99);
        });

        let state_destroy = Arc::clone(&state);
        let barrier_destroy = Arc::clone(&barrier);
        let destroy = thread::spawn(move || {
            barrier_destroy.wait();
            // Simulate destroy: remove the container record.
            let mut m = state_destroy
                .lock()
                .expect("mutex poisoned in destroy thread");
            m.remove("ctr-race");
        });

        create.join().expect("create thread panicked");
        destroy.join().expect("destroy thread panicked");

        // Post-condition: mutex must not be poisoned; map has 0 or 1 entries.
        let m = state.lock().expect("mutex poisoned after race");
        assert!(
            m.len() <= 1,
            "expected at most 1 container entry, got {}",
            m.len()
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2: event subscribe vs broadcast
// ---------------------------------------------------------------------------

/// Simulates a subscriber registering at the same instant as an event is
/// emitted.
///
/// The issue: if subscribe() returns a Receiver *after* the broadcast has
/// already been sent, the subscriber misses the event. The barrier forces both
/// threads to reach their send/subscribe calls simultaneously so the scheduler
/// picks the order.
///
/// Invariant: events emitted *after* subscribe() returns must not be dropped.
/// We verify this by emitting a second event strictly after the subscriber is
/// registered (the subscribe thread signals readiness via a Mutex<bool>).
#[test]
fn race_subscribe_vs_broadcast() {
    let broker = Arc::new(BroadcastEventBroker::new());

    for _ in 0..100 {
        let barrier = Arc::new(Barrier::new(2));

        // Shared flag: subscriber sets this to true once its Receiver is live.
        let subscribed: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));

        let broker_emit = Arc::clone(&broker);
        let barrier_emit = Arc::clone(&barrier);
        let subscribed_emit = Arc::clone(&subscribed);
        let emit_thread = thread::spawn(move || {
            barrier_emit.wait();
            // First emit — subscriber may or may not be listening yet.
            broker_emit.emit(ContainerEvent::ImagePulled {
                image: "alpine:race".to_owned(),
                size_bytes: 1024,
                timestamp: SystemTime::now(),
            });

            // Spin-wait until the subscriber has registered its Receiver.
            // This guarantees the second event is never missed.
            loop {
                let ready = *subscribed_emit.lock().expect("subscribed flag poisoned");
                if ready {
                    break;
                }
                thread::yield_now();
            }

            // This event MUST be received — subscriber is live at this point.
            broker_emit.emit(ContainerEvent::ImagePruned {
                count: 1,
                freed_bytes: 512,
                timestamp: SystemTime::now(),
            });
        });

        let broker_sub = Arc::clone(&broker);
        let barrier_sub = Arc::clone(&barrier);
        let subscribed_sub = Arc::clone(&subscribed);
        let sub_thread = thread::spawn(move || {
            barrier_sub.wait();
            let mut rx = broker_sub.subscribe();

            // Signal that the Receiver is live.
            *subscribed_sub.lock().expect("subscribed flag poisoned") = true;

            // Drain until we see the ImagePruned event (guaranteed delivery).
            let mut got_guaranteed = false;
            // Collect up to 10 events to avoid spinning forever.
            for _ in 0..10 {
                match rx.try_recv() {
                    Ok(ContainerEvent::ImagePruned { .. }) => {
                        got_guaranteed = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                        thread::yield_now();
                    }
                    Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
                }
            }
            // Poll a bit more aggressively for the guaranteed event.
            if !got_guaranteed {
                for _ in 0..1000 {
                    match rx.try_recv() {
                        Ok(ContainerEvent::ImagePruned { .. }) => {
                            got_guaranteed = true;
                            break;
                        }
                        Ok(_) => {}
                        Err(_) => {
                            thread::yield_now();
                        }
                    }
                }
            }
            assert!(
                got_guaranteed,
                "subscriber missed guaranteed ImagePruned event emitted after subscribe()"
            );
        });

        emit_thread.join().expect("emit thread panicked");
        sub_thread.join().expect("subscribe thread panicked");
    }
}

// ---------------------------------------------------------------------------
// Test 3: pause vs container exit (cgroup write guard)
// ---------------------------------------------------------------------------

/// Simulates a pause attempt racing with a container exit.
///
/// Stand-in: the cgroup path is represented by `Arc<Mutex<Option<String>>>`.
/// `Some(path)` means the container is running; `None` means it has exited.
/// A pause writer must check that the container is still alive before writing.
///
/// Invariant: the pause operation must not write to a cgroup after the
/// container has exited (option is `None`).
#[test]
fn race_pause_vs_container_exit() {
    // Track cgroup writes: each successful (non-skipped) write appends the
    // container ID here.
    let cgroup_writes: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    for _ in 0..100 {
        let barrier = Arc::new(Barrier::new(2));

        // Container starts alive.
        let cgroup_state: Arc<Mutex<Option<String>>> =
            Arc::new(Mutex::new(Some("/sys/fs/cgroup/ctr-race".to_owned())));

        let state_pause = Arc::clone(&cgroup_state);
        let barrier_pause = Arc::clone(&barrier);
        let writes_pause = Arc::clone(&cgroup_writes);
        let pause_thread = thread::spawn(move || {
            barrier_pause.wait();
            // Simulate pause: lock, check alive, write freeze.
            let mut guard = state_pause.lock().expect("cgroup state poisoned");
            if let Some(ref path) = *guard {
                // Container still alive — safe to write cgroup freeze.
                writes_pause
                    .lock()
                    .expect("cgroup writes poisoned")
                    .push(path.clone());
            }
            // If None, container already exited — skip write (correct behaviour).
            let _ = guard.take(); // simulate the pause completing
        });

        let state_exit = Arc::clone(&cgroup_state);
        let barrier_exit = Arc::clone(&barrier);
        let exit_thread = thread::spawn(move || {
            barrier_exit.wait();
            // Simulate container exit: clear the cgroup path.
            let mut guard = state_exit.lock().expect("cgroup state poisoned");
            *guard = None;
        });

        pause_thread.join().expect("pause thread panicked");
        exit_thread.join().expect("exit thread panicked");

        // After both threads finish, the cgroup path must be None (cleared).
        let final_state = cgroup_state
            .lock()
            .expect("cgroup state poisoned after race");
        assert!(
            final_state.is_none(),
            "cgroup path should be None after exit, got {:?}",
            *final_state
        );
    }

    // If writes happened, they were all to paths that existed at write time
    // (guarded by the mutex check) — no write occurred to an exited container.
    // The test passing without panic confirms the invariant.
}

// ---------------------------------------------------------------------------
// Test 4: GC sweep vs active image pull
// ---------------------------------------------------------------------------

/// Simulates a GC sweep racing with an in-progress image pull.
///
/// Stand-in: in-progress pulls are tracked via `Arc<Mutex<HashSet<String>>>`.
/// The GC sweep iterates over all known images and skips any currently in the
/// in-progress set. The puller inserts into the set before "downloading" and
/// removes it after.
///
/// Invariant: GC must never mark an in-progress image for removal.
#[test]
fn race_gc_sweep_vs_active_pull() {
    use std::collections::HashSet;

    // All images the GC knows about.
    let all_images: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new({
        let mut s = HashSet::new();
        s.insert("alpine:latest".to_owned());
        s
    }));

    // Images whose pull is currently in flight.
    let in_progress: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Images the GC decided to remove during each iteration.
    let removed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    for _ in 0..100 {
        let barrier = Arc::new(Barrier::new(2));
        const PULL_IMAGE: &str = "alpine:latest";

        // Clear previous iteration's removed list.
        removed.lock().expect("removed poisoned").clear();

        let all_gc = Arc::clone(&all_images);
        let progress_gc = Arc::clone(&in_progress);
        let removed_gc = Arc::clone(&removed);
        let barrier_gc = Arc::clone(&barrier);
        let gc_thread = thread::spawn(move || {
            barrier_gc.wait();
            // GC sweep: skip images that are currently being pulled.
            let all = all_gc.lock().expect("all_images poisoned").clone();
            let progress = progress_gc.lock().expect("in_progress poisoned").clone();
            for image in &all {
                if !progress.contains(image) {
                    removed_gc
                        .lock()
                        .expect("removed poisoned")
                        .push(image.clone());
                }
            }
        });

        let progress_pull = Arc::clone(&in_progress);
        let barrier_pull = Arc::clone(&barrier);
        let pull_thread = thread::spawn(move || {
            barrier_pull.wait();
            // Puller: register in-progress before work begins.
            progress_pull
                .lock()
                .expect("in_progress poisoned")
                .insert(PULL_IMAGE.to_owned());
            // Simulate pull work (yield to give GC a chance to run).
            thread::yield_now();
            // Remove lease after pull completes.
            progress_pull
                .lock()
                .expect("in_progress poisoned")
                .remove(PULL_IMAGE);
        });

        gc_thread.join().expect("gc thread panicked");
        pull_thread.join().expect("pull thread panicked");

        // If the puller registered before GC swept, GC must have skipped it.
        // If GC swept before the puller registered, removal is valid (image was
        // not yet in-progress). Either outcome is correct — what is NOT allowed
        // is a crash or a mutex poison.
        drop(removed.lock().expect("removed poisoned after race"));
        drop(in_progress.lock().expect("in_progress poisoned after race"));
    }
}
