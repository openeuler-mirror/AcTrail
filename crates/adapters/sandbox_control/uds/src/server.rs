//! Server configuration, safe socket ownership, and lifecycle handle.

use std::ffi::CString;
use std::fs::{self, Permissions};
use std::io::Write;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};

use sandbox_control::SandboxControlPort;

use crate::dispatcher::Dispatcher;
use crate::runtime::ServerRuntime;
use crate::{
    SandboxControlCodec, SandboxControlConnectionLimits, SandboxControlUdsError,
    SandboxControlUdsStage,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxControlUdsServerConfig {
    socket_path: PathBuf,
    socket_mode: u32,
    accepted_connection_max: usize,
    worker_thread_stack_bytes: usize,
}

impl SandboxControlUdsServerConfig {
    pub fn new(
        socket_path: impl Into<PathBuf>,
        socket_mode: u32,
        accepted_connection_max: usize,
        worker_thread_stack_bytes: usize,
    ) -> Result<Self, SandboxControlUdsError> {
        let socket_path = socket_path.into();
        if !socket_path.is_absolute() {
            return Err(SandboxControlUdsError::new(
                SandboxControlUdsStage::Configure,
                "sandbox control socket path must be absolute",
            ));
        }
        if socket_mode > 0o777 {
            return Err(SandboxControlUdsError::new(
                SandboxControlUdsStage::Configure,
                "sandbox control socket mode must contain permission bits only",
            ));
        }
        if accepted_connection_max == 0 {
            return Err(SandboxControlUdsError::new(
                SandboxControlUdsStage::Configure,
                "sandbox control accepted connection limit must be positive",
            ));
        }
        if worker_thread_stack_bytes == 0 {
            return Err(SandboxControlUdsError::new(
                SandboxControlUdsStage::Configure,
                "sandbox control worker thread stack must be positive",
            ));
        }
        Ok(Self {
            socket_path,
            socket_mode,
            accepted_connection_max,
            worker_thread_stack_bytes,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub const fn socket_mode(&self) -> u32 {
        self.socket_mode
    }

    /// Limits accepted userspace connection owners; the kernel listen backlog is platform-owned.
    pub const fn accepted_connection_max(&self) -> usize {
        self.accepted_connection_max
    }

    pub const fn worker_thread_stack_bytes(&self) -> usize {
        self.worker_thread_stack_bytes
    }
}

pub struct SandboxControlUdsServer {
    config: SandboxControlUdsServerConfig,
    limits: SandboxControlConnectionLimits,
    codec: SandboxControlCodec,
}

impl SandboxControlUdsServer {
    pub fn new(
        config: SandboxControlUdsServerConfig,
        limits: SandboxControlConnectionLimits,
        codec: SandboxControlCodec,
    ) -> Result<Self, SandboxControlUdsError> {
        if limits.request_bytes() < codec.max_frame_bytes()
            || limits.response_bytes() < codec.max_frame_bytes()
        {
            return Err(SandboxControlUdsError::new(
                SandboxControlUdsStage::Configure,
                "sandbox control connection limits must cover the codec frame limit",
            ));
        }
        Ok(Self {
            config,
            limits,
            codec,
        })
    }

    pub fn start<S>(self, service: S) -> Result<SandboxControlServerHandle, SandboxControlUdsError>
    where
        S: SandboxControlPort,
    {
        let (listener, socket_owner) = bind_listener(&self.config)?;
        let (stop_reader, stop_writer) = control_pair()?;
        let (health_reader, health_writer) = control_pair()?;
        let dispatcher = Dispatcher::start(service, self.config.worker_thread_stack_bytes)?;
        let runtime = ServerRuntime::new(
            listener,
            socket_owner,
            stop_reader,
            dispatcher,
            self.config.accepted_connection_max,
            self.limits,
            self.codec,
        );
        let worker = thread::Builder::new()
            .name("actrail-sb-control".to_string())
            .stack_size(self.config.worker_thread_stack_bytes)
            .spawn(move || {
                let _health_lifetime = health_writer;
                runtime.run()
            })
            .map_err(|error| io_error(SandboxControlUdsStage::Configure, error))?;
        Ok(SandboxControlServerHandle {
            stop_writer: Some(stop_writer),
            health_reader,
            worker: Some(worker),
        })
    }

    pub const fn config(&self) -> &SandboxControlUdsServerConfig {
        &self.config
    }
}

pub struct SandboxControlServerHandle {
    stop_writer: Option<UnixStream>,
    health_reader: UnixStream,
    worker: Option<JoinHandle<Result<(), SandboxControlUdsError>>>,
}

impl SandboxControlServerHandle {
    /// Returns an fd that becomes readable or hung up when the poll owner exits.
    pub fn health_raw_fd(&self) -> RawFd {
        self.health_reader.as_raw_fd()
    }

    pub fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub fn try_result(&mut self) -> Option<Result<(), SandboxControlUdsError>> {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            Some(self.join())
        } else {
            None
        }
    }

    /// Stops listener admission and the poll owner; it does not join the service dispatcher.
    pub fn request_stop(&mut self) -> Result<(), SandboxControlUdsError> {
        let Some(mut writer) = self.stop_writer.take() else {
            return Ok(());
        };
        match writer.write(&[1]) {
            Ok(_) => Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(io_error(SandboxControlUdsStage::Write, error)),
        }
    }

    /// Waits only for the nonblocking poll owner after admission has been stopped.
    pub fn join(&mut self) -> Result<(), SandboxControlUdsError> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker.join().map_err(|_| {
            SandboxControlUdsError::new(
                SandboxControlUdsStage::Join,
                "sandbox control server thread panicked",
            )
        })?
    }

    pub fn shutdown(&mut self) -> Result<(), SandboxControlUdsError> {
        let stop_result = self.request_stop();
        let join_result = self.join();
        stop_result.and(join_result)
    }
}

impl Drop for SandboxControlServerHandle {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn control_pair() -> Result<(UnixStream, UnixStream), SandboxControlUdsError> {
    let (reader, writer) =
        UnixStream::pair().map_err(|error| io_error(SandboxControlUdsStage::Configure, error))?;
    reader
        .set_nonblocking(true)
        .and_then(|_| writer.set_nonblocking(true))
        .map_err(|error| io_error(SandboxControlUdsStage::Configure, error))?;
    Ok((reader, writer))
}

fn bind_listener(
    config: &SandboxControlUdsServerConfig,
) -> Result<(UnixListener, BoundSocket), SandboxControlUdsError> {
    let parent = config.socket_path.parent().ok_or_else(|| {
        SandboxControlUdsError::new(
            SandboxControlUdsStage::Bind,
            "sandbox control socket path has no parent directory",
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| io_error(SandboxControlUdsStage::Bind, error))?;
    prepare_socket_path(&config.socket_path)?;
    let private_path = private_bind_path(&config.socket_path)?;
    let listener = UnixListener::bind(&private_path)
        .map_err(|error| io_error(SandboxControlUdsStage::Bind, error))?;
    let mut socket_owner = BoundSocket::capture(private_path)?;
    fs::set_permissions(
        &socket_owner.path,
        Permissions::from_mode(config.socket_mode),
    )
    .and_then(|_| listener.set_nonblocking(true))
    .map_err(|error| io_error(SandboxControlUdsStage::Bind, error))?;
    publish_socket(&socket_owner.path, &config.socket_path)?;
    socket_owner.path = config.socket_path.clone();
    Ok((listener, socket_owner))
}

fn private_bind_path(path: &Path) -> Result<PathBuf, SandboxControlUdsError> {
    let parent = path.parent().expect("validated socket parent");
    let _name = path.file_name().ok_or_else(|| {
        SandboxControlUdsError::new(
            SandboxControlUdsStage::Bind,
            "sandbox control socket path has no file name",
        )
    })?;
    let mut nonce = 0_u64;
    let result = unsafe {
        libc::getrandom(
            (&mut nonce as *mut u64).cast::<libc::c_void>(),
            std::mem::size_of::<u64>(),
            0,
        )
    };
    if result != std::mem::size_of::<u64>() as isize {
        return Err(io_error(
            SandboxControlUdsStage::Bind,
            std::io::Error::last_os_error(),
        ));
    }
    let private_name = format!(".asb-{nonce:016x}");
    Ok(parent.join(private_name))
}

fn publish_socket(private_path: &Path, public_path: &Path) -> Result<(), SandboxControlUdsError> {
    let private = CString::new(private_path.as_os_str().as_bytes()).map_err(|_| {
        SandboxControlUdsError::new(
            SandboxControlUdsStage::Bind,
            "sandbox control private socket path contains NUL",
        )
    })?;
    let public = CString::new(public_path.as_os_str().as_bytes()).map_err(|_| {
        SandboxControlUdsError::new(
            SandboxControlUdsStage::Bind,
            "sandbox control socket path contains NUL",
        )
    })?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            private.as_ptr(),
            libc::AT_FDCWD,
            public.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io_error(
            SandboxControlUdsStage::Bind,
            std::io::Error::last_os_error(),
        ))
    }
}

fn prepare_socket_path(path: &Path) -> Result<(), SandboxControlUdsError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(SandboxControlUdsStage::Bind, error)),
    };
    if !metadata.file_type().is_socket() {
        return Err(SandboxControlUdsError::new(
            SandboxControlUdsStage::Bind,
            "sandbox control path exists and is not a Unix socket",
        ));
    }
    match UnixStream::connect(path) {
        Ok(_) => Err(SandboxControlUdsError::new(
            SandboxControlUdsStage::Bind,
            "sandbox control socket already has an active listener",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
            remove_stale_socket(path, metadata.dev(), metadata.ino())
        }
        Err(error) => Err(SandboxControlUdsError::new(
            SandboxControlUdsStage::Bind,
            format!("cannot prove existing sandbox control socket is stale: {error}"),
        )),
    }
}

fn remove_stale_socket(
    path: &Path,
    expected_device: u64,
    expected_inode: u64,
) -> Result<(), SandboxControlUdsError> {
    let current = fs::symlink_metadata(path)
        .map_err(|error| io_error(SandboxControlUdsStage::Bind, error))?;
    if !current.file_type().is_socket()
        || current.dev() != expected_device
        || current.ino() != expected_inode
    {
        return Err(SandboxControlUdsError::new(
            SandboxControlUdsStage::Bind,
            "sandbox control socket changed during stale check",
        ));
    }
    fs::remove_file(path).map_err(|error| io_error(SandboxControlUdsStage::Bind, error))
}

pub(crate) struct BoundSocket {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl BoundSocket {
    fn capture(path: PathBuf) -> Result<Self, SandboxControlUdsError> {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| io_error(SandboxControlUdsStage::Bind, error))?;
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

impl Drop for BoundSocket {
    fn drop(&mut self) {
        let Ok(metadata) = fs::symlink_metadata(&self.path) else {
            return;
        };
        if metadata.file_type().is_socket()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn io_error(stage: SandboxControlUdsStage, error: std::io::Error) -> SandboxControlUdsError {
    SandboxControlUdsError::new(stage, error.to_string())
}
