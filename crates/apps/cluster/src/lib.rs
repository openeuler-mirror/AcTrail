//! Minimal offline/periodic cluster reporting for AcTrail.

use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use config_core::daemon::{
    ClusterCenterConfig, ClusterConfig, ClusterReportConfig, DEFAULT_OPERATOR_CONFIG_PATH,
    OperatorConfig,
};
use json_graph_export::service::JsonGraphExportService;
use model_core::ids::TraceId;
use model_core::trace::{TraceHealth, TraceRecord};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use storage_core::{StorageBackend, StorageOpenMode, TraceFilter};
use storage_factory::{StorageBackendKind, StorageConfig, open_storage_backend};

const BUNDLE_MAGIC_V1: &[u8] = b"ACTRAIL-BUNDLE-V1\n";
const BUNDLE_MAGIC_V2: &[u8] = b"ACTRAIL-BUNDLE-V2\n";
const SQLITE_SNAPSHOT_FILE: &str = "trace.sqlite";

#[derive(Clone, Debug, Parser)]
#[command(
    name = "actrailcluster",
    about = "Pack, upload, receive, and index AcTrail cluster bundles"
)]
struct Cli {
    #[arg(long = "config", global = true, default_value = DEFAULT_OPERATOR_CONFIG_PATH)]
    config_path: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Debug, Subcommand)]
enum Command {
    #[command(about = "Pack one local trace into a cluster bundle")]
    Pack {
        #[arg(long = "trace-id", value_parser = parse_trace_id)]
        trace_id: TraceId,
        #[arg(long = "output")]
        output: PathBuf,
        #[arg(long = "allow-active", default_value_t = false)]
        allow_active: bool,
    },
    #[command(about = "Upload terminal traces once according to [cluster.report]")]
    UploadOnce,
    #[command(about = "Receive bundles on the center node")]
    Serve,
    #[command(about = "Import one bundle file into the center index")]
    Import {
        bundle: PathBuf,
        #[arg(long = "replace", default_value_t = false)]
        replace: bool,
    },
    #[command(about = "List imported traces from the center index")]
    List,
}

pub fn run(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let cli = Cli::try_parse_from(args).unwrap_or_else(|error| error.exit());
    match cli.command {
        Command::Pack {
            trace_id,
            output,
            allow_active,
        } => {
            let config = OperatorConfig::load(&cli.config_path)?;
            pack_trace(&config, trace_id, &output, allow_active)?;
            println!("packed {trace_id} to {}", output.display());
            Ok(())
        }
        Command::UploadOnce => {
            let config = OperatorConfig::load(&cli.config_path)?;
            upload_once(&config)
        }
        Command::Serve => {
            let config = OperatorConfig::load(&cli.config_path)?;
            serve(&config.cluster.center)
        }
        Command::Import { bundle, replace } => {
            let config = OperatorConfig::load(&cli.config_path)?;
            validate_center_config(&config.cluster.center)?;
            let status = import_bundle_file(&bundle, &config.cluster.center.root_dir, replace)?;
            println!("{status}");
            Ok(())
        }
        Command::List => {
            let config = OperatorConfig::load(&cli.config_path)?;
            validate_center_config(&config.cluster.center)?;
            list_imported(&config.cluster.center.root_dir)
        }
    }
}

fn pack_trace(
    config: &OperatorConfig,
    trace_id: TraceId,
    output: &Path,
    allow_active: bool,
) -> Result<BundleManifest, String> {
    validate_cluster_config(&config.cluster)?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create bundle directory {}: {error}", parent.display()))?;
    }
    let mut storage = open_storage(&config.storage)?;
    let trace = storage
        .get_trace(trace_id)
        .map_err(storage_error("get trace"))?
        .ok_or_else(|| format!("trace not found: {trace_id}"))?;
    if !allow_active && !trace.lifecycle_state.is_terminal() {
        return Err(format!(
            "trace {trace_id} is {}; only terminal traces can be packed without --allow-active",
            trace.lifecycle_state
        ));
    }
    let manifest = build_manifest(config, storage.as_ref(), &trace)?;
    let json_graph = export_json(config, storage.as_mut(), trace_id)?;
    let sqlite_snapshot = export_sqlite_snapshot(&config.storage, output)?;
    let bundle = BundleDocument {
        manifest: manifest.clone(),
        json_graph,
        sqlite_snapshot,
    };
    write_bundle_file(output, &bundle)?;
    Ok(manifest)
}

fn upload_once(config: &OperatorConfig) -> Result<(), String> {
    validate_cluster_config(&config.cluster)?;
    let report = &config.cluster.report;
    if !config.cluster.enabled || !report.enabled {
        return Err("cluster reporting is disabled; set [cluster].enabled=true and [cluster.report].enabled=true".to_string());
    }
    fs::create_dir_all(&report.spool_dir).map_err(|error| {
        format!(
            "create cluster report spool dir {}: {error}",
            report.spool_dir.display()
        )
    })?;
    if let Some(parent) = report
        .state_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create report state dir {}: {error}", parent.display()))?;
    }
    let state = ReportState::open(&report.state_path)?;
    let storage = open_storage(&config.storage)?;
    let traces = storage
        .list_traces(&TraceFilter::default())
        .map_err(storage_error("list traces"))?;
    let candidate_count = traces
        .iter()
        .filter(|trace| !report.terminal_only || trace.lifecycle_state.is_terminal())
        .count();
    println!(
        "upload-once: candidates={candidate_count} batch_max={} terminal_only={}",
        report.batch_max_traces, report.terminal_only
    );
    let token = read_optional_token(report.auth_token_file.as_deref())?;
    let mut uploaded = 0_u32;
    let mut skipped_unchanged = 0_u32;
    let mut visited = 0_u32;
    for trace in traces {
        if uploaded >= report.batch_max_traces {
            break;
        }
        if report.terminal_only && !trace.lifecycle_state.is_terminal() {
            continue;
        }
        visited += 1;
        let local_trace_id = trace.trace_id.to_string();
        let trace_uid = config.cluster.trace_uid(&local_trace_id);
        if trace.lifecycle_state.is_terminal()
            && state.uploaded_bundle_fingerprint(&trace_uid)?.is_some()
        {
            skipped_unchanged += 1;
            println!(
                "[{visited}/{candidate_count}] {trace_uid}: skipped terminal trace already uploaded"
            );
            continue;
        }
        let bundle_path = report
            .spool_dir
            .join(safe_bundle_file_name(&trace_uid, "actrailbundle"));
        println!("[{visited}/{candidate_count}] {trace_uid}: pack start");
        let pack_started = Instant::now();
        match pack_trace(config, trace.trace_id, &bundle_path, !report.terminal_only) {
            Ok(manifest) => {
                let bundle_bytes = fs::metadata(&bundle_path)
                    .map(|metadata| metadata.len())
                    .unwrap_or_default();
                println!(
                    "[{visited}/{candidate_count}] {}: pack done in {} bundle={} events={} actions={} payloads={}",
                    manifest.trace.trace_uid,
                    human_duration(pack_started.elapsed()),
                    human_bytes(bundle_bytes),
                    manifest.counts.events,
                    manifest.counts.semantic_actions,
                    manifest.counts.payload_segments
                );
                println!(
                    "[{visited}/{candidate_count}] {}: fingerprint start",
                    manifest.trace.trace_uid
                );
                let bundle_fingerprint = match bundle_data_fingerprint_file(&bundle_path) {
                    Ok(bundle_fingerprint) => bundle_fingerprint,
                    Err(error) => {
                        state.mark_failed(&trace_uid, &local_trace_id, &bundle_path, &error)?;
                        eprintln!("fingerprint failed for {trace_uid}: {error}");
                        continue;
                    }
                };
                println!(
                    "[{visited}/{candidate_count}] {}: fingerprint done {bundle_fingerprint}",
                    manifest.trace.trace_uid
                );
                if state.uploaded_bundle_fingerprint(&trace_uid)?.as_deref()
                    == Some(bundle_fingerprint.as_str())
                {
                    skipped_unchanged += 1;
                    println!("skipped unchanged {}", manifest.trace.trace_uid);
                    continue;
                }
                println!(
                    "[{visited}/{candidate_count}] {}: upload start",
                    manifest.trace.trace_uid
                );
                match upload_bundle(
                    report,
                    token.as_deref(),
                    &bundle_path,
                    &manifest.trace.trace_uid,
                ) {
                    Ok(response) => {
                        state.mark_uploaded(
                            &trace_uid,
                            &local_trace_id,
                            &bundle_path,
                            &bundle_fingerprint,
                            &response,
                        )?;
                        uploaded += 1;
                        println!("uploaded {} ({response})", manifest.trace.trace_uid);
                    }
                    Err(error) => {
                        state.mark_failed(&trace_uid, &local_trace_id, &bundle_path, &error)?;
                        eprintln!("upload failed for {trace_uid}: {error}");
                    }
                }
            }
            Err(error) => {
                state.mark_failed(&trace_uid, &local_trace_id, &bundle_path, &error)?;
                eprintln!("pack failed for {trace_uid}: {error}");
            }
        }
    }
    println!("upload-once completed: uploaded={uploaded} skipped_unchanged={skipped_unchanged}");
    Ok(())
}

fn serve(center: &ClusterCenterConfig) -> Result<(), String> {
    validate_center_config(center)?;
    let root_dir = &center.root_dir;
    fs::create_dir_all(root_dir)
        .map_err(|error| format!("create center root dir {}: {error}", root_dir.display()))?;
    init_center_index(root_dir)?;
    let expected_token = read_optional_token(center.auth_token_file.as_deref())?;
    let listen = format!("{}:{}", center.listen_host, center.listen_port);
    let listener = TcpListener::bind(&listen).map_err(|error| format!("bind {listen}: {error}"))?;
    println!("actrailcluster center listening on {listen}");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_connection(stream, root_dir, expected_token.as_deref()) {
                    eprintln!("request failed: {error}");
                }
            }
            Err(error) => eprintln!("accept failed: {error}"),
        }
    }
    Ok(())
}

fn handle_connection(
    mut stream: TcpStream,
    root_dir: &Path,
    expected_token: Option<&str>,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .map_err(|error| format!("set read timeout: {error}"))?;
    let request = read_http_request(&mut stream)?;
    if request.method != "POST" || request.path != "/api/v1/trace-bundles" {
        return write_http_json(&mut stream, 404, r#"{"error":"not found"}"#);
    }
    if let Some(expected) = expected_token {
        let actual = request
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.trim());
        let expected_header = format!("Bearer {expected}");
        if actual != Some(expected_header.as_str()) {
            return write_http_json(&mut stream, 401, r#"{"error":"unauthorized"}"#);
        }
    }
    let status = import_bundle_bytes(&request.body, root_dir, true)?;
    let body = format!(r#"{{"status":"{status}"}}"#);
    write_http_json(&mut stream, 200, &body)
}

fn import_bundle_file(bundle: &Path, root_dir: &Path, replace: bool) -> Result<String, String> {
    let bytes =
        fs::read(bundle).map_err(|error| format!("read bundle {}: {error}", bundle.display()))?;
    import_bundle_bytes(&bytes, root_dir, replace)
}

fn import_bundle_bytes(bytes: &[u8], root_dir: &Path, replace: bool) -> Result<String, String> {
    let bundle = parse_bundle(bytes)?;
    let trace_uid = &bundle.manifest.trace.trace_uid;
    let target_dir = bundle_dir(root_dir, trace_uid);
    if target_dir.exists() {
        if !replace {
            return Ok(format!("duplicate:{trace_uid}"));
        }
        fs::remove_dir_all(&target_dir)
            .map_err(|error| format!("replace bundle dir {}: {error}", target_dir.display()))?;
    }
    fs::create_dir_all(&target_dir)
        .map_err(|error| format!("create bundle dir {}: {error}", target_dir.display()))?;
    fs::write(target_dir.join("bundle.actrailbundle"), bytes)
        .map_err(|error| format!("write bundle: {error}"))?;
    let manifest_json = serde_json::to_string_pretty(&bundle.manifest)
        .map_err(|error| format!("encode manifest: {error}"))?;
    fs::write(target_dir.join("manifest.json"), manifest_json.as_bytes())
        .map_err(|error| format!("write manifest: {error}"))?;
    fs::write(target_dir.join("graph.json"), bundle.json_graph.as_bytes())
        .map_err(|error| format!("write graph json: {error}"))?;
    if let Some(sqlite_snapshot) = &bundle.sqlite_snapshot {
        fs::write(target_dir.join(SQLITE_SNAPSHOT_FILE), sqlite_snapshot)
            .map_err(|error| format!("write sqlite snapshot: {error}"))?;
    }
    upsert_center_index(root_dir, &bundle.manifest, &target_dir, replace)?;
    Ok(format!("imported:{trace_uid}"))
}

fn list_imported(root_dir: &Path) -> Result<(), String> {
    let connection = open_center_index(root_dir)?;
    let mut statement = connection
        .prepare(
            "select trace_uid, cluster_id, node_ip, node_id, local_trace_id, display_name, lifecycle_state, event_count, semantic_action_count, imported_at from imported_traces order by imported_at desc",
        )
        .map_err(|error| format!("prepare list imported traces: {error}"))?;
    let mut rows = statement
        .query([])
        .map_err(|error| format!("query imported traces: {error}"))?;
    println!(
        "{:<40} {:<12} {:<16} {:<16} {:<10} {:<10} {:<8} {:<8} imported_at",
        "trace_uid", "cluster", "node_ip", "node_id", "state", "name", "events", "actions"
    );
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("read imported trace row: {error}"))?
    {
        let trace_uid: String = row.get(0).map_err(sql_error("trace_uid"))?;
        let cluster_id: String = row.get(1).map_err(sql_error("cluster_id"))?;
        let node_ip: String = row.get(2).map_err(sql_error("node_ip"))?;
        let node_id: String = row.get(3).map_err(sql_error("node_id"))?;
        let display_name: String = row.get(5).map_err(sql_error("display_name"))?;
        let lifecycle_state: String = row.get(6).map_err(sql_error("lifecycle_state"))?;
        let event_count: i64 = row.get(7).map_err(sql_error("event_count"))?;
        let semantic_action_count: i64 = row.get(8).map_err(sql_error("semantic_action_count"))?;
        let imported_at: String = row.get(9).map_err(sql_error("imported_at"))?;
        println!(
            "{:<40} {:<12} {:<16} {:<16} {:<10} {:<10} {:<8} {:<8} {}",
            trace_uid,
            cluster_id,
            node_ip,
            node_id,
            lifecycle_state,
            truncate(&display_name, 10),
            event_count,
            semantic_action_count,
            imported_at
        );
    }
    Ok(())
}

fn validate_cluster_config(config: &ClusterConfig) -> Result<(), String> {
    if !config.enabled {
        return Err("cluster is disabled; set [cluster].enabled=true".to_string());
    }
    for (key, value) in [
        ("cluster.cluster_id", &config.cluster_id),
        ("cluster.node_id", &config.node_id),
        ("cluster.node_name", &config.node_name),
        ("cluster.node_ip", &config.node_ip),
    ] {
        if value.trim().is_empty() {
            return Err(format!("missing {key}"));
        }
    }
    Ok(())
}

fn validate_center_config(config: &ClusterCenterConfig) -> Result<(), String> {
    if !config.enabled {
        return Err("cluster center is disabled; set [cluster.center].enabled=true".to_string());
    }
    if config.listen_host.trim().is_empty() {
        return Err("missing cluster.center.listen_host".to_string());
    }
    if config.listen_port == 0 {
        return Err("missing cluster.center.listen_port".to_string());
    }
    Ok(())
}

fn open_storage(config: &StorageConfig) -> Result<Box<dyn StorageBackend>, String> {
    open_storage_backend(config, StorageOpenMode::ReadOnly)
        .map_err(|error| format!("open storage {}: {}", error.stage, error.message))
}

fn build_manifest(
    config: &OperatorConfig,
    storage: &dyn StorageBackend,
    trace: &TraceRecord,
) -> Result<BundleManifest, String> {
    let trace_id = trace.trace_id;
    let event_counts = storage
        .count_events_by_variant(trace_id)
        .map_err(storage_error("count events"))?;
    let event_count = event_counts.values().sum::<usize>() as u64;
    let process_count = storage
        .trace_memberships(trace_id)
        .map_err(storage_error("trace memberships"))?
        .len() as u64;
    let payload_segment_count = storage
        .count_payload_segments(trace_id)
        .map_err(storage_error("count payload segments"))? as u64;
    let retained_payload_bytes = storage
        .retained_payload_bytes(trace_id)
        .map_err(storage_error("retained payload bytes"))?;
    let diagnostic_count = storage
        .list_diagnostics(trace_id)
        .map_err(storage_error("list diagnostics"))?
        .len() as u64;
    let semantic_summary = storage
        .semantic_action_summary(trace_id)
        .map_err(storage_error("semantic summary"))?;
    let local_trace_id = trace_id.to_string();
    Ok(BundleManifest {
        format: "actrail.trace.bundle.v1".to_string(),
        created_unix_ms: unix_ms(SystemTime::now()),
        cluster: BundleCluster {
            cluster_id: config.cluster.cluster_id.clone(),
            node_id: config.cluster.node_id.clone(),
            node_name: config.cluster.node_name.clone(),
            node_ip: config.cluster.node_ip.clone(),
        },
        trace: BundleTrace {
            local_trace_id: local_trace_id.clone(),
            trace_uid: config.cluster.trace_uid(&local_trace_id),
            display_name: trace.display_name.as_str().to_string(),
            profile_name: trace.profile_name.as_str().to_string(),
            lifecycle_state: trace.lifecycle_state.as_storage_str().to_string(),
            health: trace_health_as_str(trace.health).to_string(),
            created_unix_ms: unix_ms(trace.timings.created_at),
            started_unix_ms: trace.timings.started_at.map(unix_ms),
            completed_unix_ms: trace.timings.completed_at.map(unix_ms),
            root_container_id: trace.root_container_id.clone(),
        },
        counts: BundleCounts {
            processes: process_count,
            events: event_count,
            payload_segments: payload_segment_count,
            retained_payload_bytes,
            semantic_actions: semantic_summary.actions as u64,
            semantic_links: semantic_summary.links as u64,
            diagnostics: diagnostic_count,
        },
    })
}

fn export_json(
    config: &OperatorConfig,
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<String, String> {
    let mut exporter = JsonGraphExportService::new(
        storage,
        config.export_config.graph_schema_version.clone(),
        config.export_config.payload_bytes_enabled,
        config.export_config.payload_text_enabled,
    );
    exporter
        .export_json(trace_id)
        .map_err(|error| format!("export json failed: {}: {}", error.stage, error.message))
}

fn export_sqlite_snapshot(
    storage_config: &StorageConfig,
    bundle_output: &Path,
) -> Result<Option<Vec<u8>>, String> {
    match storage_config.backend() {
        StorageBackendKind::Sqlite => {}
    }
    let source = storage_config.path();
    if !source.exists() {
        return Err(format!(
            "sqlite storage path does not exist: {}",
            source.display()
        ));
    }
    let snapshot_path = sqlite_snapshot_temp_path(bundle_output);
    if snapshot_path.exists() {
        fs::remove_file(&snapshot_path).map_err(|error| {
            format!(
                "remove stale sqlite snapshot {}: {error}",
                snapshot_path.display()
            )
        })?;
    }
    let connection = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| {
        format!(
            "open sqlite storage for snapshot {}: {error}",
            source.display()
        )
    })?;
    let destination = snapshot_path.to_string_lossy().to_string();
    connection
        .execute("VACUUM main INTO ?1", params![destination])
        .map_err(|error| {
            format!(
                "create sqlite snapshot {}: {error}",
                snapshot_path.display()
            )
        })?;
    drop(connection);
    let bytes = fs::read(&snapshot_path)
        .map_err(|error| format!("read sqlite snapshot {}: {error}", snapshot_path.display()))?;
    fs::remove_file(&snapshot_path).map_err(|error| {
        format!(
            "remove sqlite snapshot temp file {}: {error}",
            snapshot_path.display()
        )
    })?;
    Ok(Some(bytes))
}

fn sqlite_snapshot_temp_path(bundle_output: &Path) -> PathBuf {
    let name = bundle_output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bundle");
    bundle_output.with_file_name(format!("{name}.sqlite-snapshot.tmp"))
}

fn bundle_data_fingerprint_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("read bundle for fingerprint {}: {error}", path.display()))?;
    let bundle = parse_bundle(&bytes)?;
    let manifest = &bundle.manifest;
    let mut hash = FNV1A64_OFFSET_BASIS;
    fnv1a64_update(&mut hash, manifest.format.as_bytes());
    fnv1a64_update_json(&mut hash, &manifest.cluster)?;
    fnv1a64_update_json(&mut hash, &manifest.trace)?;
    fnv1a64_update_json(&mut hash, &manifest.counts)?;
    fnv1a64_update(&mut hash, bundle.json_graph.as_bytes());
    if let Some(sqlite_snapshot) = &bundle.sqlite_snapshot {
        fnv1a64_update(&mut hash, sqlite_snapshot);
    }
    Ok(format!("fnv1a64:{hash:016x}"))
}

const FNV1A64_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325_u64;
const FNV1A64_PRIME: u64 = 0x0000_0100_0000_01b3_u64;

fn fnv1a64_update(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV1A64_PRIME);
    }
}

fn fnv1a64_update_json<T: Serialize>(hash: &mut u64, value: &T) -> Result<(), String> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| format!("encode fingerprint json: {error}"))?;
    fnv1a64_update(hash, &bytes);
    Ok(())
}

fn upload_bundle(
    report: &ClusterReportConfig,
    token: Option<&str>,
    bundle_path: &Path,
    trace_uid: &str,
) -> Result<String, String> {
    if report.scheme != "http" {
        return Err(format!(
            "unsupported cluster.report.scheme {}; only http is supported in phase 1",
            report.scheme
        ));
    }
    let total_bytes = fs::metadata(bundle_path)
        .map_err(|error| format!("stat bundle {}: {error}", bundle_path.display()))?
        .len();
    let mut bundle_file = fs::File::open(bundle_path)
        .map_err(|error| format!("open bundle {}: {error}", bundle_path.display()))?;
    let address = format!("{}:{}", report.center_host, report.center_port);
    let mut stream =
        TcpStream::connect(&address).map_err(|error| format!("connect {address}: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(report.upload_timeout_secs)))
        .map_err(|error| format!("set upload timeout: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(report.upload_timeout_secs)))
        .map_err(|error| format!("set response timeout: {error}"))?;
    let mut request = format!(
        "POST /api/v1/trace-bundles HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/vnd.actrail.bundle\r\nContent-Length: {}\r\nConnection: close\r\n",
        total_bytes
    );
    if let Some(token) = token {
        request.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("write upload headers: {error}"))?;
    let progress = UploadProgress::new(trace_uid, total_bytes);
    progress
        .emit(0, false)
        .map_err(|error| format!("write upload progress: {error}"))?;
    let upload_started = Instant::now();
    let mut sent = 0_u64;
    let mut last_progress_at = Instant::now();
    let mut last_progress_bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = bundle_file
            .read(&mut buffer)
            .map_err(|error| format!("read bundle {}: {error}", bundle_path.display()))?;
        if read == 0 {
            break;
        }
        stream
            .write_all(&buffer[..read])
            .map_err(|error| format!("write upload body: {error}"))?;
        sent += read as u64;
        let finished = sent == total_bytes;
        if finished
            || sent.saturating_sub(last_progress_bytes) >= 1024 * 1024
            || last_progress_at.elapsed() >= Duration::from_millis(500)
        {
            progress
                .emit(sent, finished)
                .map_err(|error| format!("write upload progress: {error}"))?;
            last_progress_at = Instant::now();
            last_progress_bytes = sent;
        }
    }
    if sent != total_bytes {
        return Err(format!(
            "upload body size mismatch: sent {} expected {}",
            sent, total_bytes
        ));
    }
    eprintln!(
        "upload {trace_uid}: sent {} in {}, waiting center import response",
        human_bytes(sent),
        human_duration(upload_started.elapsed())
    );
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("read upload response: {error}"))?;
    let mut lines = response.lines();
    let status = lines.next().unwrap_or_default();
    if !status.contains(" 200 ") {
        return Err(format!("upload rejected: {status}; {response}"));
    }
    Ok(response)
}

struct UploadProgress<'a> {
    trace_uid: &'a str,
    total_bytes: u64,
    started: Instant,
}

impl<'a> UploadProgress<'a> {
    fn new(trace_uid: &'a str, total_bytes: u64) -> Self {
        Self {
            trace_uid,
            total_bytes,
            started: Instant::now(),
        }
    }

    fn emit(&self, sent_bytes: u64, finished: bool) -> io::Result<()> {
        let percent = if self.total_bytes == 0 {
            100.0
        } else {
            (sent_bytes as f64 / self.total_bytes as f64) * 100.0
        };
        let elapsed = self.started.elapsed().as_secs_f64().max(0.001);
        let bytes_per_sec = (sent_bytes as f64 / elapsed) as u64;
        let bar = progress_bar(sent_bytes, self.total_bytes, 24);
        let mut stderr = io::stderr().lock();
        write!(
            stderr,
            "\rupload {} [{}] {percent:5.1}% {}/{} {}/s",
            self.trace_uid,
            bar,
            human_bytes(sent_bytes),
            human_bytes(self.total_bytes),
            human_bytes(bytes_per_sec)
        )?;
        if finished {
            writeln!(stderr)?;
        }
        stderr.flush()
    }
}

fn progress_bar(done: u64, total: u64, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let filled = if total == 0 {
        width
    } else {
        ((done as f64 / total as f64) * width as f64).round() as usize
    }
    .min(width);
    format!("{}{}", "=".repeat(filled), "-".repeat(width - filled))
}

fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn human_duration(duration: Duration) -> String {
    let millis = duration.as_millis();
    if millis >= 60_000 {
        format!("{:.1}s", duration.as_secs_f64())
    } else if millis >= 1_000 {
        format!("{:.2}s", duration.as_secs_f64())
    } else {
        format!("{millis}ms")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BundleDocument {
    manifest: BundleManifest,
    json_graph: String,
    sqlite_snapshot: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BundleManifest {
    format: String,
    created_unix_ms: u128,
    cluster: BundleCluster,
    trace: BundleTrace,
    counts: BundleCounts,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BundleCluster {
    cluster_id: String,
    node_id: String,
    node_name: String,
    node_ip: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BundleTrace {
    local_trace_id: String,
    trace_uid: String,
    display_name: String,
    profile_name: String,
    lifecycle_state: String,
    health: String,
    created_unix_ms: u128,
    started_unix_ms: Option<u128>,
    completed_unix_ms: Option<u128>,
    root_container_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BundleCounts {
    processes: u64,
    events: u64,
    payload_segments: u64,
    retained_payload_bytes: u64,
    semantic_actions: u64,
    semantic_links: u64,
    diagnostics: u64,
}

fn write_bundle_file(path: &Path, bundle: &BundleDocument) -> Result<(), String> {
    let mut bytes = Vec::new();
    let manifest = serde_json::to_vec(&bundle.manifest)
        .map_err(|error| format!("encode bundle manifest: {error}"))?;
    let graph = bundle.json_graph.as_bytes();
    let sqlite = bundle.sqlite_snapshot.as_deref().unwrap_or(&[]);
    bytes.extend_from_slice(BUNDLE_MAGIC_V2);
    bytes.extend_from_slice(
        format!("{}\n{}\n{}\n", manifest.len(), graph.len(), sqlite.len()).as_bytes(),
    );
    bytes.extend_from_slice(&manifest);
    bytes.extend_from_slice(graph);
    bytes.extend_from_slice(sqlite);
    fs::write(path, bytes).map_err(|error| format!("write bundle {}: {error}", path.display()))
}

fn parse_bundle(bytes: &[u8]) -> Result<BundleDocument, String> {
    if bytes.starts_with(BUNDLE_MAGIC_V2) {
        return parse_bundle_v2(bytes);
    }
    if bytes.starts_with(BUNDLE_MAGIC_V1) {
        return parse_bundle_v1(bytes);
    }
    Err("invalid bundle magic".to_string())
}

fn parse_bundle_v1(bytes: &[u8]) -> Result<BundleDocument, String> {
    let mut cursor = BUNDLE_MAGIC_V1.len();
    let manifest_len = read_len_line(bytes, &mut cursor, "manifest length")?;
    let graph_len = read_len_line(bytes, &mut cursor, "graph length")?;
    let manifest_end = cursor
        .checked_add(manifest_len)
        .ok_or_else(|| "bundle manifest length overflow".to_string())?;
    let graph_end = manifest_end
        .checked_add(graph_len)
        .ok_or_else(|| "bundle graph length overflow".to_string())?;
    if graph_end != bytes.len() {
        return Err("bundle length mismatch".to_string());
    }
    let manifest = serde_json::from_slice(&bytes[cursor..manifest_end])
        .map_err(|error| format!("decode bundle manifest: {error}"))?;
    let json_graph = String::from_utf8(bytes[manifest_end..graph_end].to_vec())
        .map_err(|error| format!("decode graph json: {error}"))?;
    Ok(BundleDocument {
        manifest,
        json_graph,
        sqlite_snapshot: None,
    })
}

fn parse_bundle_v2(bytes: &[u8]) -> Result<BundleDocument, String> {
    if !bytes.starts_with(BUNDLE_MAGIC_V2) {
        return Err("invalid bundle magic".to_string());
    }
    let mut cursor = BUNDLE_MAGIC_V2.len();
    let manifest_len = read_len_line(bytes, &mut cursor, "manifest length")?;
    let graph_len = read_len_line(bytes, &mut cursor, "graph length")?;
    let sqlite_len = read_len_line(bytes, &mut cursor, "sqlite snapshot length")?;
    let manifest_end = cursor
        .checked_add(manifest_len)
        .ok_or_else(|| "bundle manifest length overflow".to_string())?;
    let graph_end = manifest_end
        .checked_add(graph_len)
        .ok_or_else(|| "bundle graph length overflow".to_string())?;
    let sqlite_end = graph_end
        .checked_add(sqlite_len)
        .ok_or_else(|| "bundle sqlite snapshot length overflow".to_string())?;
    if sqlite_end != bytes.len() {
        return Err("bundle length mismatch".to_string());
    }
    let manifest = serde_json::from_slice(&bytes[cursor..manifest_end])
        .map_err(|error| format!("decode bundle manifest: {error}"))?;
    let json_graph = String::from_utf8(bytes[manifest_end..graph_end].to_vec())
        .map_err(|error| format!("decode graph json: {error}"))?;
    let sqlite_snapshot = if sqlite_len == 0 {
        None
    } else {
        Some(bytes[graph_end..sqlite_end].to_vec())
    };
    Ok(BundleDocument {
        manifest,
        json_graph,
        sqlite_snapshot,
    })
}

fn read_len_line(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<usize, String> {
    let start = *cursor;
    while *cursor < bytes.len() && bytes[*cursor] != b'\n' {
        *cursor += 1;
    }
    if *cursor >= bytes.len() {
        return Err(format!("missing {label}"));
    }
    let raw = std::str::from_utf8(&bytes[start..*cursor])
        .map_err(|error| format!("decode {label}: {error}"))?;
    *cursor += 1;
    raw.parse::<usize>()
        .map_err(|error| format!("parse {label}: {error}"))
}

struct ReportState {
    connection: Connection,
}

impl ReportState {
    fn open(path: &Path) -> Result<Self, String> {
        let connection = Connection::open(path)
            .map_err(|error| format!("open report state {}: {error}", path.display()))?;
        connection
            .execute_batch(
                "create table if not exists report_state (
                    trace_uid text primary key,
                    local_trace_id text not null,
                    bundle_path text,
                    bundle_fingerprint text,
                    status text not null,
                    attempt_count integer not null default 0,
                    last_attempt_at text,
                    uploaded_at text,
                    last_error text,
                    last_response text
                );",
            )
            .map_err(|error| format!("initialize report state: {error}"))?;
        ensure_report_state_column(&connection, "bundle_fingerprint", "text")?;
        Ok(Self { connection })
    }

    fn uploaded_bundle_fingerprint(&self, trace_uid: &str) -> Result<Option<String>, String> {
        self
            .connection
            .query_row(
                "select bundle_fingerprint from report_state where trace_uid = ?1 and status = 'uploaded'",
                params![trace_uid],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|value| value.flatten())
            .map_err(|error| format!("query report state: {error}"))
    }

    fn mark_uploaded(
        &self,
        trace_uid: &str,
        local_trace_id: &str,
        bundle_path: &Path,
        bundle_fingerprint: &str,
        response: &str,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "insert into report_state(trace_uid, local_trace_id, bundle_path, bundle_fingerprint, status, attempt_count, last_attempt_at, uploaded_at, last_error, last_response)
                 values(?1, ?2, ?3, ?4, 'uploaded', 1, ?5, ?5, null, ?6)
                 on conflict(trace_uid) do update set
                   bundle_path=excluded.bundle_path,
                   bundle_fingerprint=excluded.bundle_fingerprint,
                   status='uploaded',
                   attempt_count=report_state.attempt_count + 1,
                   last_attempt_at=excluded.last_attempt_at,
                   uploaded_at=excluded.uploaded_at,
                   last_error=null,
                   last_response=excluded.last_response",
                params![
                    trace_uid,
                    local_trace_id,
                    bundle_path.display().to_string(),
                    bundle_fingerprint,
                    unix_ms(SystemTime::now()).to_string(),
                    response
                ],
            )
            .map_err(|error| format!("mark uploaded: {error}"))?;
        Ok(())
    }

    fn mark_failed(
        &self,
        trace_uid: &str,
        local_trace_id: &str,
        bundle_path: &Path,
        error: &str,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "insert into report_state(trace_uid, local_trace_id, bundle_path, status, attempt_count, last_attempt_at, uploaded_at, last_error, last_response)
                 values(?1, ?2, ?3, 'failed', 1, ?4, null, ?5, null)
                 on conflict(trace_uid) do update set
                   bundle_path=excluded.bundle_path,
                   status='failed',
                   attempt_count=report_state.attempt_count + 1,
                   last_attempt_at=excluded.last_attempt_at,
                   last_error=excluded.last_error",
                params![
                    trace_uid,
                    local_trace_id,
                    bundle_path.display().to_string(),
                    unix_ms(SystemTime::now()).to_string(),
                    error
                ],
            )
            .map_err(|error| format!("mark failed: {error}"))?;
        Ok(())
    }
}

fn ensure_report_state_column(
    connection: &Connection,
    column_name: &str,
    column_type: &str,
) -> Result<(), String> {
    let mut statement = connection
        .prepare("select name from pragma_table_info('report_state') where name = ?1")
        .map_err(|error| format!("prepare report_state column check: {error}"))?;
    let exists = statement
        .query_row(params![column_name], |_| Ok(()))
        .optional()
        .map_err(|error| format!("query report_state column check: {error}"))?
        .is_some();
    if exists {
        return Ok(());
    }
    let sql = format!("alter table report_state add column {column_name} {column_type}");
    connection
        .execute(&sql, [])
        .map_err(|error| format!("alter report_state add {column_name}: {error}"))?;
    Ok(())
}

fn init_center_index(root_dir: &Path) -> Result<(), String> {
    let connection = open_center_index(root_dir)?;
    connection
        .execute_batch(
            "create table if not exists imported_traces (
                trace_uid text primary key,
                cluster_id text not null,
                node_ip text not null,
                node_id text not null,
                node_name text,
                local_trace_id text not null,
                display_name text,
                profile_name text,
                lifecycle_state text not null,
                health text not null,
                created_unix_ms text,
                started_unix_ms text,
                completed_unix_ms text,
                imported_at text not null,
                bundle_path text not null,
                graph_json_path text not null,
                process_count integer not null,
                event_count integer not null,
                payload_segment_count integer not null,
                retained_payload_bytes integer not null,
                semantic_action_count integer not null,
                semantic_link_count integer not null,
                diagnostic_count integer not null,
                manifest_json text not null
            );
            create index if not exists idx_imported_traces_cluster_node on imported_traces(cluster_id, node_ip, node_id);
            create index if not exists idx_imported_traces_started_at on imported_traces(started_unix_ms);",
        )
        .map_err(|error| format!("initialize center index: {error}"))
}

fn open_center_index(root_dir: &Path) -> Result<Connection, String> {
    fs::create_dir_all(root_dir)
        .map_err(|error| format!("create center root dir {}: {error}", root_dir.display()))?;
    let path = root_dir.join("index.sqlite");
    Connection::open(&path)
        .map_err(|error| format!("open center index {}: {error}", path.display()))
}

fn upsert_center_index(
    root_dir: &Path,
    manifest: &BundleManifest,
    target_dir: &Path,
    replace: bool,
) -> Result<(), String> {
    init_center_index(root_dir)?;
    let connection = open_center_index(root_dir)?;
    let manifest_json = serde_json::to_string(manifest)
        .map_err(|error| format!("encode manifest for index: {error}"))?;
    let sql = if replace {
        "insert into imported_traces(trace_uid, cluster_id, node_ip, node_id, node_name, local_trace_id, display_name, profile_name, lifecycle_state, health, created_unix_ms, started_unix_ms, completed_unix_ms, imported_at, bundle_path, graph_json_path, process_count, event_count, payload_segment_count, retained_payload_bytes, semantic_action_count, semantic_link_count, diagnostic_count, manifest_json)
         values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)
         on conflict(trace_uid) do update set
           cluster_id=excluded.cluster_id,
           node_ip=excluded.node_ip,
           node_id=excluded.node_id,
           node_name=excluded.node_name,
           local_trace_id=excluded.local_trace_id,
           display_name=excluded.display_name,
           profile_name=excluded.profile_name,
           lifecycle_state=excluded.lifecycle_state,
           health=excluded.health,
           created_unix_ms=excluded.created_unix_ms,
           started_unix_ms=excluded.started_unix_ms,
           completed_unix_ms=excluded.completed_unix_ms,
           imported_at=excluded.imported_at,
           bundle_path=excluded.bundle_path,
           graph_json_path=excluded.graph_json_path,
           process_count=excluded.process_count,
           event_count=excluded.event_count,
           payload_segment_count=excluded.payload_segment_count,
           retained_payload_bytes=excluded.retained_payload_bytes,
           semantic_action_count=excluded.semantic_action_count,
           semantic_link_count=excluded.semantic_link_count,
           diagnostic_count=excluded.diagnostic_count,
           manifest_json=excluded.manifest_json"
    } else {
        "insert into imported_traces(trace_uid, cluster_id, node_ip, node_id, node_name, local_trace_id, display_name, profile_name, lifecycle_state, health, created_unix_ms, started_unix_ms, completed_unix_ms, imported_at, bundle_path, graph_json_path, process_count, event_count, payload_segment_count, retained_payload_bytes, semantic_action_count, semantic_link_count, diagnostic_count, manifest_json)
         values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)"
    };
    connection
        .execute(
            sql,
            params![
                manifest.trace.trace_uid,
                manifest.cluster.cluster_id,
                manifest.cluster.node_ip,
                manifest.cluster.node_id,
                manifest.cluster.node_name,
                manifest.trace.local_trace_id,
                manifest.trace.display_name,
                manifest.trace.profile_name,
                manifest.trace.lifecycle_state,
                manifest.trace.health,
                manifest.trace.created_unix_ms.to_string(),
                manifest
                    .trace
                    .started_unix_ms
                    .map(|value| value.to_string()),
                manifest
                    .trace
                    .completed_unix_ms
                    .map(|value| value.to_string()),
                unix_ms(SystemTime::now()).to_string(),
                target_dir
                    .join("bundle.actrailbundle")
                    .display()
                    .to_string(),
                target_dir.join("graph.json").display().to_string(),
                manifest.counts.processes as i64,
                manifest.counts.events as i64,
                manifest.counts.payload_segments as i64,
                manifest.counts.retained_payload_bytes as i64,
                manifest.counts.semantic_actions as i64,
                manifest.counts.semantic_links as i64,
                manifest.counts.diagnostics as i64,
                manifest_json
            ],
        )
        .map_err(|error| format!("insert center index: {error}"))?;
    Ok(())
}

struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|error| format!("read request: {error}"))?;
        if read == 0 {
            return Err("connection closed before headers".to_string());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 64 * 1024 {
            return Err("request headers too large".to_string());
        }
        if let Some(pos) = find_bytes(&buffer, b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers_raw = std::str::from_utf8(&buffer[..header_end])
        .map_err(|error| format!("decode headers: {error}"))?;
    let mut lines = headers_raw.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "missing request line".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut headers = Vec::new();
    let mut content_length = 0_usize;
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.eq_ignore_ascii_case("content-length") {
            content_length = value
                .trim()
                .parse::<usize>()
                .map_err(|error| format!("invalid content-length: {error}"))?;
        }
        headers.push((key.trim().to_string(), value.trim().to_string()));
    }
    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let read_size = remaining.min(chunk.len());
        let read = stream
            .read(&mut chunk[..read_size])
            .map_err(|error| format!("read body: {error}"))?;
        if read == 0 {
            return Err("connection closed before body complete".to_string());
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn write_http_json(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| format!("write response: {error}"))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn read_optional_token(path: Option<&Path>) -> Result<Option<String>, String> {
    path.map(|path| {
        fs::read_to_string(path)
            .map_err(|error| format!("read token file {}: {error}", path.display()))
            .map(|token| token.trim().to_string())
    })
    .transpose()
}

fn parse_trace_id(raw: &str) -> Result<TraceId, String> {
    let number = raw
        .strip_prefix("trace-")
        .ok_or_else(|| "trace id must be formatted as trace-<number>".to_string())?;
    number
        .parse::<u64>()
        .map(TraceId::new)
        .map_err(|error| format!("invalid trace id: {error}"))
}

fn storage_error(stage: &'static str) -> impl FnOnce(storage_core::StorageError) -> String {
    move |error| format!("{stage}: {}: {}", error.stage, error.message)
}

fn sql_error(field: &'static str) -> impl FnOnce(rusqlite::Error) -> String {
    move |error| format!("read {field}: {error}")
}

fn trace_health_as_str(value: TraceHealth) -> &'static str {
    match value {
        TraceHealth::Clean => "clean",
        TraceHealth::Degraded => "degraded",
    }
}

fn unix_ms(value: SystemTime) -> u128 {
    value
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn safe_bundle_file_name(trace_uid: &str, extension: &str) -> String {
    format!("{}.{}", sanitize_path_segment(trace_uid), extension)
}

fn bundle_dir(root_dir: &Path, trace_uid: &str) -> PathBuf {
    root_dir
        .join("bundles")
        .join(sanitize_path_segment(trace_uid))
}

fn sanitize_path_segment(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>()
        + "…"
}
