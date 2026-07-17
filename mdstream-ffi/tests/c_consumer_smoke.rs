use std::{
    env::consts::{DLL_PREFIX, DLL_SUFFIX, EXE_SUFFIX},
    path::{Path, PathBuf},
    process::Command,
};

#[path = "support/host.rs"]
mod host_support;

use host_support::{TempDir, current_target};

#[test]
fn external_c_consumer_links_and_runs_against_the_dynamic_library() {
    compile_and_run(Linkage::Dynamic);
}

#[test]
fn external_c_consumer_links_and_runs_against_the_exact_static_archive() {
    compile_and_run(Linkage::Static);
}

#[derive(Debug, Clone, Copy)]
enum Linkage {
    Dynamic,
    Static,
}

fn compile_and_run(linkage: Linkage) {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let artifact_dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let out_dir = TempDir::new(&format!("mdstream-ffi-c-consumer-{linkage:?}"));
    let executable = out_dir.path().join(executable_name("mdstream-c-consumer"));
    let library = artifact_dir.join(match linkage {
        Linkage::Dynamic => dynamic_library_name(),
        Linkage::Static => static_library_name(),
    });
    assert!(
        library.is_file(),
        "Cargo did not build the expected {} artifact",
        library.display()
    );

    let mut build = cc::Build::new();
    build
        .target(current_target())
        .host(current_target())
        .opt_level(0);
    let compiler = build.get_compiler();
    assert!(
        !compiler.is_like_msvc(),
        "MSVC real-link smoke needs an import-library-specific lane"
    );
    let mut command = compiler.to_command();
    command
        .arg("-std=c11")
        .arg("-I")
        .arg(manifest_dir.join("include"))
        .arg(manifest_dir.join("tests/c_consumer_smoke.c"))
        .arg(&library);
    match linkage {
        Linkage::Dynamic => {
            command.arg(format!("-Wl,-rpath,{}", artifact_dir.display()));
        }
        Linkage::Static => add_native_static_libraries(&mut command),
    }
    command.arg("-o").arg(&executable);
    run_command(command, "compile external C consumer");

    let mut smoke = Command::new(&executable);
    add_dynamic_library_path(&mut smoke, &artifact_dir);
    run_command(smoke, "run external C consumer");
}

fn add_native_static_libraries(command: &mut Command) {
    if cfg!(target_os = "macos") {
        command.args(["-lSystem", "-lc", "-lm"]);
    } else if cfg!(target_os = "linux") {
        command.args(["-ldl", "-lpthread", "-lm", "-lrt", "-lutil"]);
    }
}

fn add_dynamic_library_path(command: &mut Command, directory: &Path) {
    if cfg!(target_os = "macos") {
        command.env("DYLD_LIBRARY_PATH", directory);
    } else if cfg!(target_os = "linux") {
        command.env("LD_LIBRARY_PATH", directory);
    }
}

fn dynamic_library_name() -> String {
    format!("{DLL_PREFIX}mdstream_ffi{DLL_SUFFIX}")
}

fn static_library_name() -> String {
    if cfg!(target_os = "windows") {
        "mdstream_ffi.lib".to_string()
    } else {
        "libmdstream_ffi.a".to_string()
    }
}

fn executable_name(stem: &str) -> String {
    format!("{stem}{EXE_SUFFIX}")
}

fn run_command(mut command: Command, action: &str) {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("failed to {action}: {error}"));
    assert!(
        output.status.success(),
        "failed to {action}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
