use super::slot_table;
use super::targets::TlsFuncKind;
use super::wrappers;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum BindingSource {
    Audit,
    Resolver,
}

pub(in crate::runtime) fn get_or_create_bound_wrapper(
    kind: TlsFuncKind,
    real_sym: usize,
    _source: BindingSource,
) -> Option<usize> {
    if real_sym == 0 || wrappers::is_managed_entry(real_sym) {
        return Some(real_sym);
    }
    let slot = slot_table::get_or_create_slot(kind, real_sym)?;
    Some(wrappers::entry_for_slot(kind, slot))
}

pub(in crate::runtime) fn real_symbol_for_slot(kind: TlsFuncKind, slot: usize) -> Option<usize> {
    slot_table::real_symbol_for_slot(kind, slot)
}
