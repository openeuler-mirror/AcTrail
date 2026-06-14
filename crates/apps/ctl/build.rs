use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let source_dir = manifest_dir.join("java-agent/src/main/java");
    println!("cargo:rerun-if-changed={}", source_dir.display());
    println!("cargo:rerun-if-env-changed=ACTRAIL_SKIP_JAVA_AGENT_BUILD");
    println!("cargo:rerun-if-env-changed=ACTRAIL_REQUIRE_JAVA_AGENT_BUILD");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set"));
    let artifact_rs = out_dir.join("java_agent_artifact.rs");
    if env::var_os("ACTRAIL_SKIP_JAVA_AGENT_BUILD").is_some() {
        write_artifact_unavailable(
            &artifact_rs,
            "Java payload agent build was skipped by ACTRAIL_SKIP_JAVA_AGENT_BUILD",
        );
        return;
    }
    let require_java_agent = env::var_os("ACTRAIL_REQUIRE_JAVA_AGENT_BUILD").is_some();
    match build_java_agent(&source_dir, &out_dir) {
        Ok(jar) => write_artifact_available(&artifact_rs, &jar),
        Err(error) => {
            let message = format!(
                "build embedded Java payload agent: {error}. \
                 Use JDK 17+ on PATH, or set ACTRAIL_SKIP_JAVA_AGENT_BUILD=1 \
                 when Java JSSE payload capture is intentionally unavailable."
            );
            if require_java_agent {
                panic!("{message}");
            }
            println!(
                "cargo:warning=Java JSSE payload capture disabled for this build: {message} \
                 Set ACTRAIL_REQUIRE_JAVA_AGENT_BUILD=1 to make this failure fatal."
            );
            write_artifact_unavailable(&artifact_rs, &message);
        }
    }
}

fn build_java_agent(source_dir: &Path, out_dir: &Path) -> Result<PathBuf, String> {
    let sources = java_sources(source_dir)?;
    if sources.is_empty() {
        return Err(format!(
            "no Java agent sources under {}",
            source_dir.display()
        ));
    }
    let classes_dir = out_dir.join("java-agent/classes");
    let jar_path = out_dir.join("actrail-java-payload-agent.jar");
    let manifest_path = out_dir.join("java-agent/MANIFEST.MF");
    recreate_dir(&classes_dir)?;
    fs::create_dir_all(
        manifest_path
            .parent()
            .ok_or_else(|| "manifest path has no parent".to_string())?,
    )
    .map_err(|error| format!("create Java agent manifest dir: {error}"))?;
    fs::write(
        &manifest_path,
        concat!(
            "Manifest-Version: 1.0\n",
            "Premain-Class: com.actrail.javaagent.AcTrailJavaPayloadAgent\n",
            "Can-Redefine-Classes: true\n",
            "Can-Retransform-Classes: true\n",
            "\n",
        ),
    )
    .map_err(|error| format!("write Java agent manifest: {error}"))?;

    let mut javac_args = vec![
        OsString::from("--release"),
        OsString::from("17"),
        OsString::from("-d"),
        classes_dir.clone().into_os_string(),
    ];
    javac_args.extend(sources.iter().map(|path| OsString::from(path.as_os_str())));
    run_tool("javac", &javac_args)?;

    let jar_args = vec![
        OsString::from("cfm"),
        jar_path.clone().into_os_string(),
        manifest_path.into_os_string(),
        OsString::from("-C"),
        classes_dir.into_os_string(),
        OsString::from("."),
    ];
    run_tool("jar", &jar_args)?;
    Ok(jar_path)
}

fn java_sources(source_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut sources = Vec::new();
    collect_java_sources(source_dir, &mut sources)?;
    sources.sort();
    Ok(sources)
}

fn collect_java_sources(dir: &Path, sources: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|error| format!("read {}: {error}", dir.display()))? {
        let entry = entry.map_err(|error| format!("read {}: {error}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_java_sources(&path, sources)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("java") {
            println!("cargo:rerun-if-changed={}", path.display());
            sources.push(path);
        }
    }
    Ok(())
}

fn recreate_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|error| format!("remove {}: {error}", path.display()))?;
    }
    fs::create_dir_all(path).map_err(|error| format!("create {}: {error}", path.display()))
}

fn run_tool(program: &str, args: &[OsString]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("run {program}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "{program} failed with status {}: {}{}{}",
        output.status,
        stdout.trim(),
        if stdout.trim().is_empty() || stderr.trim().is_empty() {
            ""
        } else {
            "\n"
        },
        stderr.trim()
    ))
}

fn write_artifact_available(path: &Path, jar: &Path) {
    let jar_literal = format!("{:?}", jar.display().to_string());
    let raw = format!(
        "pub const JAVA_PAYLOAD_AGENT_JAR: Option<&'static [u8]> = Some(include_bytes!({jar_literal}));\n\
         pub const JAVA_PAYLOAD_AGENT_BUILD_ERROR: Option<&'static str> = None;\n"
    );
    fs::write(path, raw).expect("write generated Java agent artifact descriptor");
}

fn write_artifact_unavailable(path: &Path, error: &str) {
    let error_literal = format!("{error:?}");
    let raw = format!(
        "pub const JAVA_PAYLOAD_AGENT_JAR: Option<&'static [u8]> = None;\n\
         pub const JAVA_PAYLOAD_AGENT_BUILD_ERROR: Option<&'static str> = Some({error_literal});\n"
    );
    fs::write(path, raw).expect("write generated Java agent artifact descriptor");
}
