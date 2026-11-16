#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn fast_resolves_rust_v0_rustls_impl_symbols() {
    let fixture = RustlsFixture::compile();
    let output = Command::new(env!("CARGO_BIN_EXE_tls-probe-point-finder"))
        .args(["fast", "--provider", "rustls", "--source", "executable"])
        .arg(fixture.binary())
        .output()
        .expect("run tls-probe-point-finder fast");

    assert!(
        output.status.success(),
        "finder rejected a Rust v0 rustls symbol fixture:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("finder stdout is UTF-8");
    assert!(stdout.contains("rustls_buffer_plaintext"), "{stdout}");
    assert!(
        stdout.contains("rustls_take_received_plaintext"),
        "{stdout}"
    );
}

struct RustlsFixture {
    directory: PathBuf,
    binary: PathBuf,
}

impl RustlsFixture {
    fn compile() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "actrail-rustls-symbol-fixture-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("create fixture directory");
        let source = directory.join("main.rs");
        let binary = directory.join("rustls-symbol-fixture");
        fs::write(
            &source,
            r#"
pub mod common_state {
    pub struct CommonState;

    impl CommonState {
        #[inline(never)]
        pub fn buffer_plaintext(&mut self, bytes: &[u8]) -> usize {
            std::hint::black_box(bytes.len())
        }

        #[inline(never)]
        pub fn take_received_plaintext(&mut self) -> usize {
            std::hint::black_box(0usize)
        }
    }
}

fn main() {
    let mut state = common_state::CommonState;
    std::hint::black_box(state.buffer_plaintext(b"request"));
    std::hint::black_box(state.take_received_plaintext());
}
"#,
        )
        .expect("write fixture source");
        let output = Command::new("rustc")
            .args([
                "--crate-name",
                "rustls",
                "--edition=2024",
                "-C",
                "symbol-mangling-version=v0",
                "-C",
                "strip=none",
                "-o",
            ])
            .arg(&binary)
            .arg(&source)
            .output()
            .expect("compile rustls symbol fixture");
        assert!(
            output.status.success(),
            "fixture compilation failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        Self { directory, binary }
    }

    fn binary(&self) -> &Path {
        &self.binary
    }
}

impl Drop for RustlsFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
