use std::ffi::CStr;
use std::os::raw::c_char;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum TlsFuncKind {
    SslWrite,
    SslWriteEx,
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
        match symbol {
            b"SSL_write" => Some(Self::SslWrite),
            b"SSL_write_ex" => Some(Self::SslWriteEx),
            b"SSL_read" => Some(Self::SslRead),
            b"SSL_read_ex" => Some(Self::SslReadEx),
            _ => None,
        }
    }

    pub(in crate::runtime) const fn symbol(self) -> &'static str {
        match self {
            Self::SslWrite => "SSL_write",
            Self::SslWriteEx => "SSL_write_ex",
            Self::SslRead => "SSL_read",
            Self::SslReadEx => "SSL_read_ex",
        }
    }
}
