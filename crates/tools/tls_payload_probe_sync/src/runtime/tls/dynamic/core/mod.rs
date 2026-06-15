mod binding;
pub(in crate::runtime) mod capture;
mod slot_table;
mod targets;
mod wrappers;

pub(in crate::runtime) use binding::{
    BindingSource, get_or_create_bound_wrapper, real_symbol_for_slot,
};
pub(in crate::runtime) use slot_table::SLOT_COUNT;
pub(in crate::runtime) use targets::TlsFuncKind;
