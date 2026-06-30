use std::sync::atomic::{AtomicUsize, Ordering};

use super::targets::TlsFuncKind;

pub(in crate::runtime) const SLOT_COUNT: usize = 128;

struct SlotTable {
    entries: [AtomicUsize; SLOT_COUNT],
}

impl SlotTable {
    const fn new() -> Self {
        Self {
            entries: [const { AtomicUsize::new(0) }; SLOT_COUNT],
        }
    }

    fn get_or_create(&self, real_sym: usize) -> Option<usize> {
        for (index, entry) in self.entries.iter().enumerate() {
            let current = entry.load(Ordering::Acquire);
            if current == real_sym {
                return Some(index);
            }
            if current == 0
                && entry
                    .compare_exchange(0, real_sym, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                return Some(index);
            }
        }
        None
    }

    fn real_symbol(&self, slot: usize) -> Option<usize> {
        let value = self.entries.get(slot)?.load(Ordering::Acquire);
        (value != 0).then_some(value)
    }
}

static SSL_WRITE_SLOTS: SlotTable = SlotTable::new();
static SSL_WRITE_EX_SLOTS: SlotTable = SlotTable::new();
static SSL_WRITE_EX2_SLOTS: SlotTable = SlotTable::new();
static SSL_READ_SLOTS: SlotTable = SlotTable::new();
static SSL_READ_EX_SLOTS: SlotTable = SlotTable::new();

pub(in crate::runtime) fn get_or_create_slot(kind: TlsFuncKind, real_sym: usize) -> Option<usize> {
    table(kind).get_or_create(real_sym)
}

pub(in crate::runtime) fn real_symbol_for_slot(kind: TlsFuncKind, slot: usize) -> Option<usize> {
    table(kind).real_symbol(slot)
}

fn table(kind: TlsFuncKind) -> &'static SlotTable {
    match kind {
        TlsFuncKind::SslWrite => &SSL_WRITE_SLOTS,
        TlsFuncKind::SslWriteEx => &SSL_WRITE_EX_SLOTS,
        TlsFuncKind::SslWriteEx2 => &SSL_WRITE_EX2_SLOTS,
        TlsFuncKind::SslRead => &SSL_READ_SLOTS,
        TlsFuncKind::SslReadEx => &SSL_READ_EX_SLOTS,
    }
}
