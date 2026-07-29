use std::{
    fmt::Write as _,
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn obsolete_zero_three_surface_is_not_available() {
    let project = ProbeProject::new();
    project.assert_compiles(
        "baseline",
        "use mdstream::StreamEngine; fn main() { let _ = StreamEngine::new(); }",
    );

    let symbols = [
        "AnalyzedStream",
        "AppliedUpdate",
        "Block",
        "BlockAnalyzer",
        "BlockStatus",
        "BoundaryPlugin",
        "CodeFenceHeader",
        "DocumentState",
        "MdStream",
        "MdStreamBuilder",
        "Options",
        "PendingBlockRef",
        "PendingTransformer",
        "TerminatorOptions",
        "Update",
        "UpdateRef",
        "is_code_fence_closing_line",
        "is_list_marker_line_prefix",
        "parse_code_fence_header",
        "terminate_markdown",
    ];
    project.assert_rejected(
        "obsolete symbols",
        &format!(
            "#![allow(unused_imports)] use mdstream::{{{}}}; fn main() {{}}",
            symbols.join(",")
        ),
        &symbols,
    );

    let methods = [
        ("push_boundary_plugin", "engine.push_boundary_plugin(())"),
        ("with_boundary_plugin", "engine.with_boundary_plugin(())"),
        (
            "push_pending_transformer",
            "engine.push_pending_transformer(())",
        ),
        (
            "with_pending_transformer",
            "engine.with_pending_transformer(())",
        ),
        ("append_ref", "engine.append_ref(\"text\")"),
        ("finalize_ref", "engine.finalize_ref()"),
        ("snapshot_blocks", "engine.snapshot_blocks()"),
        ("committed_mut", "engine.committed_mut()"),
    ];
    let mut method_probes = String::new();
    for (_, invocation) in &methods {
        write!(
            method_probes,
            "{{ let mut engine = StreamEngine::new(); let _ = {invocation}; }}"
        )
        .expect("writing to a String cannot fail");
    }
    let method_names = methods.map(|(name, _)| name);
    project.assert_rejected(
        "obsolete methods",
        &format!("use mdstream::StreamEngine; fn main() {{ {method_probes} }}"),
        &method_names,
    );
}

#[test]
fn intentional_zero_four_surface_is_available() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/intentional_zero_four_surface.rs");
}

#[test]
fn compiler_budgets_are_not_available_on_protocol_limits() {
    let project = ProbeProject::new();
    project.assert_rejected(
        "compiler budgets on ProtocolLimits",
        r#"
use mdstream_protocol::ProtocolLimits;

fn main() {
    let _ = ProtocolLimits {
        max_definitions: 1,
        max_definition_edges: 1,
        max_definition_metadata_bytes: 1,
        ..ProtocolLimits::default()
    };
}
"#,
        &[
            "max_definitions",
            "max_definition_edges",
            "max_definition_metadata_bytes",
        ],
    );
}

struct ProbeProject {
    root: PathBuf,
}

impl ProbeProject {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must follow the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mdstream-public-surface-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).expect("create probe crate");
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let protocol_dir = PathBuf::from(manifest_dir)
            .parent()
            .expect("mdstream crate must live in the workspace root")
            .join("mdstream-protocol");
        fs::write(
            root.join("Cargo.toml"),
            format!(
                "[package]\nname = \"mdstream-public-surface-probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\nmdstream = {{ path = {manifest_dir:?} }}\nmdstream-protocol = {{ path = {protocol_dir:?} }}\n"
            ),
        )
        .expect("write probe manifest");
        Self { root }
    }

    fn assert_compiles(&self, name: &str, source: &str) {
        let output = self.check(source);
        assert!(
            output.status.success(),
            "public surface baseline {name} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn assert_rejected(&self, label: &str, source: &str, expected_names: &[&str]) {
        let output = self.check(source);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{label} unexpectedly compiled");
        for name in expected_names {
            assert!(
                stderr.contains(name),
                "{label} did not reject {name} explicitly:\n{stderr}"
            );
        }
    }

    fn check(&self, source: &str) -> std::process::Output {
        fs::write(self.root.join("src/main.rs"), source).expect("write probe source");
        Command::new(env!("CARGO"))
            .args(["check", "--quiet", "--offline"])
            .current_dir(&self.root)
            .env("CARGO_TARGET_DIR", self.root.join("target"))
            .output()
            .expect("run cargo check for public surface probe")
    }
}

impl Drop for ProbeProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
