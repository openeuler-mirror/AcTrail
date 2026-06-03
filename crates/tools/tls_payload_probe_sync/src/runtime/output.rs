//! Runtime output helpers.

pub(super) fn event_line(text: &str) {
    write_all(libc::STDOUT_FILENO, text);
}

pub(super) fn error_line(text: &str) {
    write_all(libc::STDERR_FILENO, text);
}

fn write_all(fd: libc::c_int, text: &str) {
    let mut bytes = text.as_bytes();
    while !bytes.is_empty() {
        let written =
            unsafe { libc::write(fd, bytes.as_ptr().cast::<libc::c_void>(), bytes.len()) };
        if written <= 0 {
            return;
        }
        bytes = &bytes[written as usize..];
    }
}
