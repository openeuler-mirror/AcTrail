//! Real workload used by live eBPF verification.

#[path = "workload/file.rs"]
mod file;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::raw::c_char;
use std::os::raw::c_int;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;

use crate::args::WorkloadConfig;
use file::{FileEndpoint, MmapSharedFile, run_file_mutation_workload};

/// `pipe(2)` returns one read fd and one write fd.
const PIPE_ENDPOINT_COUNT: usize = 2;

unsafe extern "C" {
    fn pipe(pipefd: *mut c_int) -> c_int;
    fn mkfifo(pathname: *const c_char, mode: u32) -> c_int;
}

pub fn run_workload(config: WorkloadConfig) -> Result<(), String> {
    ensure_workload_paths_absent(&config)?;
    announce_waiting_workload(&config)?;

    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    read_expected_line(&mut input, &config.stdio_stdin_message)?;

    let mut exec_child = spawn_exec_child(&config)?;
    run_signal_workload(config.process_signal_number)?;
    let mut pipe_pair = PipePair::open()?;
    pipe_pair.roundtrip(config.pipe_message.as_bytes())?;
    let mut fifo = FifoEndpoint::create(&config.fifo_path, config.fifo_mode)?;
    fifo.roundtrip(config.pipe_message.as_bytes())?;
    let mut file = FileEndpoint::create(&config.file_path)?;
    file.roundtrip(config.file_message.as_bytes())?;
    let _mmap_file = config
        .mmap
        .as_ref()
        .map(MmapSharedFile::create)
        .transpose()?;
    run_file_mutation_workload(&config)?;
    let mut unix_pair = UnixSocketPair::open()?;
    unix_pair.roundtrip(config.unix_message.as_bytes())?;
    write_stdio_payloads(&config)?;

    let listener = TcpListener::bind(&config.listen_addr).map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let client_message = config.client_message.as_bytes().to_vec();
    let server_message = config.server_message.as_bytes().to_vec();
    let response_size = server_message.len();
    let (release_tx, release_rx) = mpsc::channel::<()>();

    let server_client_message = client_message.clone();
    let server_handle = std::thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        read_exact_payload(&mut stream, &server_client_message)?;
        stream
            .write_all(&server_message)
            .map_err(|error| error.to_string())?;
        release_rx.recv().map_err(|error| error.to_string())?;
        Ok(())
    });

    let mut client = TcpStream::connect(address).map_err(|error| error.to_string())?;
    client
        .write_all(&client_message)
        .map_err(|error| error.to_string())?;
    let mut response = vec![0; response_size];
    client
        .read_exact(&mut response)
        .map_err(|error| error.to_string())?;

    announce_events_ready(&config)?;
    read_expected_line(&mut input, &config.stdio_continue_message)?;
    drop(exec_child.stdin.take());
    let exec_status = exec_child.wait().map_err(|error| error.to_string())?;
    if !exec_status.success() {
        return Err(format!(
            "verification exec target {:?} exited with {exec_status}",
            config.exec_path
        ));
    }
    drop(client);
    release_tx.send(()).map_err(|error| error.to_string())?;
    server_handle
        .join()
        .map_err(|_| "server thread panicked".to_string())??;
    Ok(())
}

fn announce_waiting_workload(config: &WorkloadConfig) -> Result<(), String> {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    writeln!(stdout, "workload_pid={}", std::process::id()).map_err(|error| error.to_string())?;
    writeln!(stdout, "waiting_for={}", config.stdio_stdin_message)
        .map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())
}

fn announce_events_ready(config: &WorkloadConfig) -> Result<(), String> {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    writeln!(stdout, "events-ready").map_err(|error| error.to_string())?;
    writeln!(stdout, "waiting_for={}", config.stdio_continue_message)
        .map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())
}

fn ensure_workload_paths_absent(config: &WorkloadConfig) -> Result<(), String> {
    let mut paths = vec![
        config.fifo_path.as_path(),
        config.file_path.as_path(),
        config.mkdir_path.as_path(),
        config.rmdir_path.as_path(),
        config.rename_source_path.as_path(),
        config.rename_target_path.as_path(),
        config.unlink_path.as_path(),
        config.truncate_path.as_path(),
    ];
    if let Some(mmap) = &config.mmap {
        paths.push(mmap.path.as_path());
    }
    for path in paths {
        if path.exists() {
            return Err(format!(
                "workload path already exists: {}; choose unused paths in the workload config or remove it after confirming no workload is using it",
                path.display()
            ));
        }
    }
    Ok(())
}

fn write_stdio_payloads(config: &WorkloadConfig) -> Result<(), String> {
    {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        writeln!(stdout, "{}", config.stdio_stdout_message).map_err(|error| error.to_string())?;
        stdout.flush().map_err(|error| error.to_string())?;
    }
    {
        let stderr = std::io::stderr();
        let mut stderr = stderr.lock();
        writeln!(stderr, "{}", config.stdio_stderr_message).map_err(|error| error.to_string())?;
        stderr.flush().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn run_signal_workload(signal_number: u32) -> Result<(), String> {
    let signal = c_int::try_from(signal_number).map_err(|error| error.to_string())?;
    if signal <= 0 {
        return Err("process signal number must be positive".to_string());
    }
    let previous = unsafe { libc::signal(signal, libc::SIG_IGN) };
    if previous == libc::SIG_ERR {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let kill_result = unsafe { libc::kill(libc::getpid(), signal) };
    let restore_result = unsafe { libc::signal(signal, previous) };
    if restore_result == libc::SIG_ERR {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if kill_result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

struct PipePair {
    reader: std::fs::File,
    writer: std::fs::File,
}

impl PipePair {
    fn open() -> Result<Self, String> {
        let mut fds = [0; PIPE_ENDPOINT_COUNT];
        let result = unsafe { pipe(fds.as_mut_ptr()) };
        if result != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let [read_fd, write_fd] = fds;
        let reader = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(read_fd) });
        let writer = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(write_fd) });
        Ok(Self { reader, writer })
    }

    fn roundtrip(&mut self, message: &[u8]) -> Result<(), String> {
        self.writer
            .write_all(message)
            .map_err(|error| error.to_string())?;
        self.writer.flush().map_err(|error| error.to_string())?;
        let mut observed = vec![0; message.len()];
        self.reader
            .read_exact(&mut observed)
            .map_err(|error| error.to_string())?;
        if observed != message {
            return Err("pipe observed unexpected payload".to_string());
        }
        Ok(())
    }
}

struct FifoEndpoint {
    file: std::fs::File,
}

impl FifoEndpoint {
    fn create(path: &Path, mode: u32) -> Result<Self, String> {
        let raw_path = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| "fifo path contains NUL byte".to_string())?;
        let result = unsafe { mkfifo(raw_path.as_ptr(), mode) };
        if result != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        Ok(Self { file })
    }

    fn roundtrip(&mut self, message: &[u8]) -> Result<(), String> {
        self.file
            .write_all(message)
            .map_err(|error| error.to_string())?;
        self.file.flush().map_err(|error| error.to_string())?;
        let mut observed = vec![0; message.len()];
        self.file
            .read_exact(&mut observed)
            .map_err(|error| error.to_string())?;
        if observed != message {
            return Err("fifo observed unexpected payload".to_string());
        }
        Ok(())
    }
}

struct UnixSocketPair {
    reader: UnixStream,
    writer: UnixStream,
}

impl UnixSocketPair {
    fn open() -> Result<Self, String> {
        let (reader, writer) = UnixStream::pair().map_err(|error| error.to_string())?;
        Ok(Self { reader, writer })
    }

    fn roundtrip(&mut self, message: &[u8]) -> Result<(), String> {
        self.writer
            .write_all(message)
            .map_err(|error| error.to_string())?;
        self.writer.flush().map_err(|error| error.to_string())?;
        let mut observed = vec![0; message.len()];
        self.reader
            .read_exact(&mut observed)
            .map_err(|error| error.to_string())?;
        if observed != message {
            return Err("unix socket observed unexpected payload".to_string());
        }
        Ok(())
    }
}

fn spawn_exec_child(config: &WorkloadConfig) -> Result<std::process::Child, String> {
    let mut child = Command::new(&config.exec_path)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to execute {:?}: {error}", config.exec_path))?;
    if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
        return Err(format!(
            "verification exec target {:?} exited before live drain with {status}",
            config.exec_path
        ));
    }
    Ok(child)
}

fn read_expected_line(input: &mut impl BufRead, expected: &str) -> Result<(), String> {
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    if line.trim_end() != expected {
        return Err(format!("expected control line {expected}, got {line:?}"));
    }
    Ok(())
}

fn read_exact_payload(stream: &mut TcpStream, expected: &[u8]) -> Result<(), String> {
    let mut observed = vec![0; expected.len()];
    stream
        .read_exact(&mut observed)
        .map_err(|error| error.to_string())?;
    if observed != expected {
        return Err("server observed unexpected client payload".to_string());
    }
    Ok(())
}
