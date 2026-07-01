use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const PREBUILT_JAVA_AGENT_ENV: &str = "ACTRAIL_JAVA_AGENT_PREBUILT_JAR";
const SKIP_JAVA_AGENT_ENV: &str = "ACTRAIL_SKIP_JAVA_AGENT_BUILD";
const REQUIRE_JAVA_AGENT_ENV: &str = "ACTRAIL_REQUIRE_JAVA_AGENT_BUILD";
const JAVA_RELEASE_ENV: &str = "ACTRAIL_JAVA_AGENT_RELEASE";
const JAVA_HOME_ENV: &str = "JAVA_HOME";
const JAVAC_ENV: &str = "ACTRAIL_JAVAC";
const JAR_ENV: &str = "ACTRAIL_JAR";
const JAVA_AGENT_SOURCE_DIR: &str = "java-agent/src/main/java";
const JAVA_AGENT_BUILD_DIR: &str = "java-agent";
const JAVA_AGENT_JAR_NAME: &str = "actrail-java-payload-agent.jar";
const DEFAULT_JAVA_RELEASE: &str = "17";

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let source_dir = manifest_dir.join(JAVA_AGENT_SOURCE_DIR);
    println!("cargo:rerun-if-changed={}", source_dir.display());
    println!("cargo:rerun-if-env-changed={PREBUILT_JAVA_AGENT_ENV}");
    println!("cargo:rerun-if-env-changed={SKIP_JAVA_AGENT_ENV}");
    println!("cargo:rerun-if-env-changed={REQUIRE_JAVA_AGENT_ENV}");
    println!("cargo:rerun-if-env-changed={JAVA_RELEASE_ENV}");
    println!("cargo:rerun-if-env-changed={JAVA_HOME_ENV}");
    println!("cargo:rerun-if-env-changed={JAVAC_ENV}");
    println!("cargo:rerun-if-env-changed={JAR_ENV}");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set"));
    let artifact_rs = out_dir.join("java_agent_artifact.rs");
    if env::var_os(SKIP_JAVA_AGENT_ENV).is_some() {
        write_artifact_unavailable(
            &artifact_rs,
            "Java payload agent build was skipped by ACTRAIL_SKIP_JAVA_AGENT_BUILD",
        );
        return;
    }

    if let Some(path) = env::var_os(PREBUILT_JAVA_AGENT_ENV) {
        match copy_prebuilt_java_agent(PathBuf::from(path), &out_dir.join(JAVA_AGENT_JAR_NAME)) {
            Ok(jar) => write_artifact_available(&artifact_rs, &jar),
            Err(error) => panic!("use prebuilt Java payload agent jar: {error}"),
        }
        return;
    }

    let require_java_agent = env::var_os(REQUIRE_JAVA_AGENT_ENV).is_some();
    let java_release = java_release();
    match build_java_agent(&source_dir, &out_dir, &java_release) {
        Ok(jar) => write_artifact_available(&artifact_rs, &jar),
        Err(error) => {
            let message = format!(
                "build embedded Java payload agent: {error}. \
                 Use JDK 17+ via JAVA_HOME, {JAVAC_ENV}/{JAR_ENV}, or PATH; \
                 set ACTRAIL_SKIP_JAVA_AGENT_BUILD=1 when Java JSSE payload \
                 capture is intentionally unavailable."
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

fn java_release() -> OsString {
    env::var_os(JAVA_RELEASE_ENV)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from(DEFAULT_JAVA_RELEASE))
}

fn copy_prebuilt_java_agent(source: PathBuf, target: &Path) -> Result<PathBuf, String> {
    if !source.is_absolute() {
        return Err(format!(
            "{PREBUILT_JAVA_AGENT_ENV} must be an absolute path"
        ));
    }
    println!("cargo:rerun-if-changed={}", source.display());
    if !source.is_file() {
        return Err(format!(
            "missing prebuilt Java payload agent jar {}",
            source.display()
        ));
    }
    println!(
        "cargo:warning=using prebuilt Java payload agent jar from {}",
        source.display()
    );
    let parent = target
        .parent()
        .ok_or_else(|| "prebuilt Java payload agent target has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "create prebuilt Java payload agent target dir {}: {error}",
            parent.display()
        )
    })?;
    fs::copy(&source, target).map_err(|error| {
        format!(
            "copy prebuilt Java payload agent jar {} to {}: {error}",
            source.display(),
            target.display()
        )
    })?;
    Ok(target.to_path_buf())
}

fn build_java_agent(
    source_dir: &Path,
    out_dir: &Path,
    java_release: &OsString,
) -> Result<PathBuf, String> {
    let sources = java_sources(source_dir)?;
    if sources.is_empty() {
        return Err(format!(
            "no Java agent sources under {}",
            source_dir.display()
        ));
    }
    let classes_dir = out_dir.join(JAVA_AGENT_BUILD_DIR).join("classes");
    let jar_path = out_dir.join(JAVA_AGENT_JAR_NAME);
    let manifest_path = out_dir.join(JAVA_AGENT_BUILD_DIR).join("MANIFEST.MF");
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
        java_release.clone(),
        OsString::from("-d"),
        classes_dir.clone().into_os_string(),
    ];
    javac_args.extend(sources.iter().map(|path| OsString::from(path.as_os_str())));
    run_tool(&java_tool("javac", JAVAC_ENV)?, &javac_args)?;

    let jar_args = vec![
        OsString::from("cfm"),
        jar_path.clone().into_os_string(),
        manifest_path.into_os_string(),
        OsString::from("-C"),
        classes_dir.into_os_string(),
        OsString::from("."),
    ];
    run_tool(&java_tool("jar", JAR_ENV)?, &jar_args)?;
    Ok(jar_path)
}

fn java_tool(program: &str, override_env: &str) -> Result<OsString, String> {
    java_tool_from_config(
        program,
        env::var_os(override_env),
        env::var_os(JAVA_HOME_ENV),
    )
}

fn java_tool_from_config(
    program: &str,
    explicit_tool: Option<OsString>,
    java_home: Option<OsString>,
) -> Result<OsString, String> {
    if let Some(tool) = explicit_tool.filter(|value| !value.is_empty()) {
        let path = PathBuf::from(&tool);
        if !path.is_absolute() {
            return Err(format!(
                "explicit Java tool path for {program} must be absolute: {}",
                path.display()
            ));
        }
        return Ok(tool);
    }
    match java_home.filter(|value| !value.is_empty()) {
        Some(home) => Ok(PathBuf::from(home)
            .join("bin")
            .join(program)
            .into_os_string()),
        None => Ok(OsString::from(program)),
    }
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

fn run_tool(program: &OsString, args: &[OsString]) -> Result<(), String> {
    let program_label = program.to_string_lossy();
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("run {program_label}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "{program_label} failed with status {}: {}{}{}",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_tool_uses_java_home_bin() {
        let tool = java_tool_from_config("javac", None, Some(OsString::from("/opt/jdk-17")))
            .expect("java home tool path");

        assert_eq!(PathBuf::from(tool), PathBuf::from("/opt/jdk-17/bin/javac"));
    }

    #[test]
    fn java_tool_ignores_empty_java_home() {
        let tool =
            java_tool_from_config("jar", None, Some(OsString::new())).expect("path fallback tool");

        assert_eq!(tool, OsString::from("jar"));
    }
}
