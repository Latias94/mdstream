use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

mod support;

use mdstream::{BoundaryUpdate, FnBoundaryPlugin, MdStream, Options};

#[test]
fn fn_boundary_plugin_can_model_a_custom_fence() {
    let started = Arc::new(AtomicUsize::new(0));
    let just_started = Arc::new(AtomicBool::new(false));

    let started_for_start = started.clone();
    let just_started_for_start = just_started.clone();
    let just_started_for_update = just_started.clone();

    let plugin = FnBoundaryPlugin::new(
        |line| line.trim() == "@@@",
        move |line| {
            if just_started_for_update.swap(false, Ordering::SeqCst) {
                return BoundaryUpdate::Continue;
            }
            if line.trim() == "@@@" {
                BoundaryUpdate::Close
            } else {
                BoundaryUpdate::Continue
            }
        },
    )
    .with_start(move |_| {
        started_for_start.fetch_add(1, Ordering::SeqCst);
        just_started_for_start.store(true, Ordering::SeqCst);
    })
    .with_reset({
        let just_started = just_started.clone();
        move || {
            just_started.store(false, Ordering::SeqCst);
        }
    });

    let markdown = "Intro\n\n@@@\nA\n\nB\n@@@\n\nAfter\n";
    let blocks = support::collect_final_raw_with_stream(
        support::chunk_whole(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(plugin),
    );
    assert_eq!(
        blocks,
        vec![
            "Intro\n\n".to_string(),
            "@@@\nA\n\nB\n@@@\n".to_string(),
            "After\n".to_string(),
        ]
    );
    assert_eq!(started.load(Ordering::SeqCst), 1);
}

#[test]
fn fn_boundary_plugin_chunking_invariance() {
    let just_started = Arc::new(AtomicBool::new(false));
    let js_start = just_started.clone();
    let js_update = just_started.clone();

    let plugin = FnBoundaryPlugin::new(
        |line| line.trim() == "@@@",
        move |line| {
            if js_update.swap(false, Ordering::SeqCst) {
                return BoundaryUpdate::Continue;
            }
            if line.trim() == "@@@" {
                BoundaryUpdate::Close
            } else {
                BoundaryUpdate::Continue
            }
        },
    )
    .with_start(move |_| {
        js_start.store(true, Ordering::SeqCst);
    });

    let markdown = "Intro\n\n@@@\nA\n\nB\n@@@\n\nAfter\n";

    let blocks_whole = support::collect_final_raw_with_stream(
        support::chunk_whole(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(plugin),
    );

    let just_started = Arc::new(AtomicBool::new(false));
    let js_start = just_started.clone();
    let js_update = just_started.clone();
    let plugin = FnBoundaryPlugin::new(
        |line| line.trim() == "@@@",
        move |line| {
            if js_update.swap(false, Ordering::SeqCst) {
                return BoundaryUpdate::Continue;
            }
            if line.trim() == "@@@" {
                BoundaryUpdate::Close
            } else {
                BoundaryUpdate::Continue
            }
        },
    )
    .with_start(move |_| {
        js_start.store(true, Ordering::SeqCst);
    });
    let blocks_lines = support::collect_final_raw_with_stream(
        support::chunk_lines(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(plugin),
    );

    let just_started = Arc::new(AtomicBool::new(false));
    let js_start = just_started.clone();
    let js_update = just_started.clone();
    let plugin = FnBoundaryPlugin::new(
        |line| line.trim() == "@@@",
        move |line| {
            if js_update.swap(false, Ordering::SeqCst) {
                return BoundaryUpdate::Continue;
            }
            if line.trim() == "@@@" {
                BoundaryUpdate::Close
            } else {
                BoundaryUpdate::Continue
            }
        },
    )
    .with_start(move |_| {
        js_start.store(true, Ordering::SeqCst);
    });
    let blocks_chars = support::collect_final_raw_with_stream(
        support::chunk_chars(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(plugin),
    );

    let just_started = Arc::new(AtomicBool::new(false));
    let js_start = just_started.clone();
    let js_update = just_started.clone();
    let plugin = FnBoundaryPlugin::new(
        |line| line.trim() == "@@@",
        move |line| {
            if js_update.swap(false, Ordering::SeqCst) {
                return BoundaryUpdate::Continue;
            }
            if line.trim() == "@@@" {
                BoundaryUpdate::Close
            } else {
                BoundaryUpdate::Continue
            }
        },
    )
    .with_start(move |_| {
        js_start.store(true, Ordering::SeqCst);
    });
    let blocks_rand = support::collect_final_raw_with_stream(
        support::chunk_pseudo_random(markdown, "fn_boundary_plugin_chunking_invariance", 0, 40),
        MdStream::new(Options::default()).with_boundary_plugin(plugin),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_chars, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);
}
