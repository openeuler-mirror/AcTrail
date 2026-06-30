mod binding;
pub(in crate::runtime) mod capture;
mod slot_table;
mod targets;
mod wrappers;

pub(in crate::runtime) use binding::{
    BindingSource, get_or_create_bound_wrapper, real_symbol_for_slot,
};
pub(in crate::runtime) use slot_table::SLOT_COUNT;
pub(in crate::runtime) use targets::{
    OPENSSL_SSL_READ, OPENSSL_SSL_READ_EX, OPENSSL_SSL_READ_EX_NUL, OPENSSL_SSL_READ_NUL,
    OPENSSL_SSL_WRITE, OPENSSL_SSL_WRITE_EX, OPENSSL_SSL_WRITE_EX_NUL, OPENSSL_SSL_WRITE_EX2,
    OPENSSL_SSL_WRITE_EX2_NUL, OPENSSL_SSL_WRITE_NUL, TlsFuncKind,
};
