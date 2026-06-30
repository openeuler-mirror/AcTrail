use std::ffi::CStr;
use std::os::raw::c_char;

pub(in crate::runtime) const OPENSSL_SSL_WRITE: &str = "SSL_write";
pub(in crate::runtime) const OPENSSL_SSL_WRITE_EX: &str = "SSL_write_ex";
pub(in crate::runtime) const OPENSSL_SSL_WRITE_EX2: &str = "SSL_write_ex2";
pub(in crate::runtime) const OPENSSL_SSL_READ: &str = "SSL_read";
pub(in crate::runtime) const OPENSSL_SSL_READ_EX: &str = "SSL_read_ex";

pub(in crate::runtime) const OPENSSL_SSL_WRITE_NUL: &[u8] = b"SSL_write\0";
pub(in crate::runtime) const OPENSSL_SSL_WRITE_EX_NUL: &[u8] = b"SSL_write_ex\0";
pub(in crate::runtime) const OPENSSL_SSL_WRITE_EX2_NUL: &[u8] = b"SSL_write_ex2\0";
pub(in crate::runtime) const OPENSSL_SSL_READ_NUL: &[u8] = b"SSL_read\0";
pub(in crate::runtime) const OPENSSL_SSL_READ_EX_NUL: &[u8] = b"SSL_read_ex\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum TlsFuncKind {
    SslWrite,
    SslWriteEx,
    SslWriteEx2,
    SslRead,
    SslReadEx,
}

impl TlsFuncKind {
    pub(in crate::runtime) fn from_c_symbol(symbol: *const c_char) -> Option<Self> {
        if symbol.is_null() {
            return None;
        }
        let symbol = unsafe { CStr::from_ptr(symbol) }.to_bytes();
        Self::from_symbol_bytes(symbol)
    }

    pub(in crate::runtime) fn from_symbol_bytes(symbol: &[u8]) -> Option<Self> {
        if symbol == OPENSSL_SSL_WRITE.as_bytes() {
            Some(Self::SslWrite)
        } else if symbol == OPENSSL_SSL_WRITE_EX.as_bytes() {
            Some(Self::SslWriteEx)
        } else if symbol == OPENSSL_SSL_WRITE_EX2.as_bytes() {
            Some(Self::SslWriteEx2)
        } else if symbol == OPENSSL_SSL_READ.as_bytes() {
            Some(Self::SslRead)
        } else if symbol == OPENSSL_SSL_READ_EX.as_bytes() {
            Some(Self::SslReadEx)
        } else {
            None
        }
    }

    pub(in crate::runtime) const fn symbol(self) -> &'static str {
        match self {
            Self::SslWrite => OPENSSL_SSL_WRITE,
            Self::SslWriteEx => OPENSSL_SSL_WRITE_EX,
            Self::SslWriteEx2 => OPENSSL_SSL_WRITE_EX2,
            Self::SslRead => OPENSSL_SSL_READ,
            Self::SslReadEx => OPENSSL_SSL_READ_EX,
        }
    }
}
