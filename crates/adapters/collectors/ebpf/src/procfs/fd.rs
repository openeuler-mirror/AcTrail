//! File-descriptor target classification through `/proc/<pid>/fd`.

use std::collections::BTreeMap;
use std::os::unix::fs::FileTypeExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FdTargetKind {
    RegularFile,
    Pipe,
    Fifo,
    UnixSocket,
    Socket,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FdObservation {
    pub kind: FdTargetKind,
    pub target: String,
    pub metadata: BTreeMap<String, String>,
}

pub fn resolve_fd_observation(pid: u32, fd: u32) -> Result<Option<FdObservation>, String> {
    let path = format!("/proc/{pid}/fd/{fd}");
    let target = match std::fs::read_link(&path) {
        Ok(target) => target.display().to_string(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };

    if target.starts_with("pipe:[") {
        return Ok(Some(FdObservation {
            kind: FdTargetKind::Pipe,
            target,
            metadata: BTreeMap::new(),
        }));
    }
    if target.starts_with("socket:[") {
        let inode = socket_inode(&target)?;
        if let Some(metadata) = unix_socket_metadata(pid, inode)? {
            return Ok(Some(FdObservation {
                kind: FdTargetKind::UnixSocket,
                target,
                metadata,
            }));
        }
        return Ok(Some(FdObservation {
            kind: FdTargetKind::Socket,
            target,
            metadata: BTreeMap::new(),
        }));
    }

    let kind = match std::fs::metadata(&path) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_fifo() {
                FdTargetKind::Fifo
            } else if file_type.is_file() {
                FdTargetKind::RegularFile
            } else {
                FdTargetKind::Other
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };

    Ok(Some(FdObservation {
        kind,
        target,
        metadata: BTreeMap::new(),
    }))
}

fn socket_inode(target: &str) -> Result<u64, String> {
    target
        .strip_prefix("socket:[")
        .and_then(|value| value.strip_suffix(']'))
        .ok_or_else(|| format!("invalid socket fd target {target}"))?
        .parse::<u64>()
        .map_err(|error| error.to_string())
}

fn unix_socket_metadata(pid: u32, inode: u64) -> Result<Option<BTreeMap<String, String>>, String> {
    let raw = match std::fs::read_to_string(format!("/proc/{pid}/net/unix")) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut lines = raw.lines();
    let Some(header) = lines.next() else {
        return Ok(None);
    };
    let headers = header.split_whitespace().collect::<Vec<_>>();
    let Some(inode_index) = headers.iter().position(|field| *field == "Inode") else {
        return Ok(None);
    };
    let path_index = headers.iter().position(|field| *field == "Path");

    for line in lines {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let Some(entry_inode) = fields
            .get(inode_index)
            .and_then(|value| value.parse::<u64>().ok())
        else {
            continue;
        };
        if entry_inode != inode {
            continue;
        }
        let mut metadata = BTreeMap::from([("unix_socket_inode".to_string(), inode.to_string())]);
        if let Some(path) = path_index.and_then(|index| fields.get(index)) {
            metadata.insert("unix_socket_path".to_string(), (*path).to_string());
        }
        return Ok(Some(metadata));
    }

    Ok(None)
}
