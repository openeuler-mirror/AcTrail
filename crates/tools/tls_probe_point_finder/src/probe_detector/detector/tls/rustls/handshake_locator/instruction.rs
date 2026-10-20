//! Small, conservative AArch64 integer instruction model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Origin {
    Unknown,
    Constant(u64),
    Argument(u8, i64),
    Reference(u8, i64),
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Instruction {
    Move(u8, u8, bool),
    Constant(u8, u64),
    Add(u8, u8, i64, bool),
    Load {
        target: u8,
        base: u8,
        offset: u16,
        bytes: u8,
    },
    StoreByte {
        value: u8,
        base: u8,
        offset: u16,
    },
    CompareOne(u8),
    ZeroBranch {
        register: u8,
        target: u64,
        zero: bool,
    },
    Branch {
        target: u64,
        conditional: bool,
    },
    Call(u64),
    IndirectCall,
    Return,
    Stop,
    Write(u8),
    Memory {
        first: Option<u8>,
        second: Option<u8>,
        writeback: Option<u8>,
    },
    NoWrite,
    Unknown,
}

impl Instruction {
    pub(super) fn decode(word: u32, pc: u64) -> Self {
        let rd = (word & 31) as u8;
        let rn = ((word >> 5) & 31) as u8;
        let rm = ((word >> 16) & 31) as u8;
        let wide = word >> 31 != 0;
        let offset = ((word >> 10) & 0xfff) as u16;
        if word & 0xfc000000 == 0x94000000 {
            return Self::Call(Self::relative(pc, word & 0x3ffffff, 26));
        }
        if word & 0xfc000000 == 0x14000000 {
            return Self::Branch {
                target: Self::relative(pc, word & 0x3ffffff, 26),
                conditional: false,
            };
        }
        if word & 0xff000010 == 0x54000000 {
            return Self::Branch {
                target: Self::relative(pc, (word >> 5) & 0x7ffff, 19),
                conditional: true,
            };
        }
        if word & 0x7e000000 == 0x34000000 {
            return Self::ZeroBranch {
                register: rd,
                target: Self::relative(pc, (word >> 5) & 0x7ffff, 19),
                zero: word & 0x1000000 == 0,
            };
        }
        if word & 0x7e000000 == 0x36000000 {
            return Self::Branch {
                target: Self::relative(pc, (word >> 5) & 0x3fff, 14),
                conditional: true,
            };
        }
        if word & 0xfffffc1f == 0xd65f0000 {
            return Self::Return;
        }
        if word & 0xfffffc1f == 0xd63f0000 {
            return Self::IndirectCall;
        }
        if word & 0xfffffc1f == 0xd61f0000 || word & 0xffe0001f == 0xd4200000 {
            return Self::Stop;
        }
        if word & 0x7fe0ffe0 == 0x2a0003e0 {
            return Self::Move(rd, rm, wide);
        }
        if word & 0x7f800000 == 0x52800000 {
            return Self::Constant(
                rd,
                u64::from((word >> 5) & 0xffff) << (((word >> 21) & 3) * 16),
            );
        }
        if word & 0x7f00001f == 0x7100001f && offset == 1 && word & (1 << 22) == 0 {
            return Self::CompareOne(rn);
        }
        if word & 0x1f800000 == 0x11000000 && word & (1 << 29) == 0 {
            let amount = i64::from(offset) << if word & (1 << 22) != 0 { 12 } else { 0 };
            return Self::Add(
                rd,
                rn,
                if word & (1 << 30) != 0 {
                    -amount
                } else {
                    amount
                },
                wide,
            );
        }
        match word & 0xffc00000 {
            0x39000000 => {
                return Self::StoreByte {
                    value: rd,
                    base: rn,
                    offset,
                };
            }
            0x39400000 => {
                return Self::Load {
                    target: rd,
                    base: rn,
                    offset,
                    bytes: 1,
                };
            }
            0xf9400000 => {
                return Self::Load {
                    target: rd,
                    base: rn,
                    offset: offset * 8,
                    bytes: 8,
                };
            }
            _ => {}
        }
        // Integer load/store pairs; writeback invalidates the old base origin.
        if word & 0x3a000000 == 0x28000000 && word & (1 << 26) == 0 {
            let load = word & (1 << 22) != 0;
            let mode = (word >> 23) & 3;
            return Self::Memory {
                first: load.then_some(rd),
                second: load.then_some(((word >> 10) & 31) as u8),
                writeback: (mode == 1 || mode == 3).then_some(rn),
            };
        }
        // Integer immediate-address loads/stores whose values are not modeled.
        if word & 0x3b000000 == 0x39000000 && word & (1 << 26) == 0 {
            return Self::Memory {
                first: ((word >> 22) & 3 != 0).then_some(rd),
                second: None,
                writeback: None,
            };
        }
        if word & 0x3b200000 == 0x38000000 && word & (1 << 26) == 0 {
            let mode = (word >> 10) & 3;
            return Self::Memory {
                first: ((word >> 22) & 3 != 0).then_some(rd),
                second: None,
                writeback: (mode == 1 || mode == 3).then_some(rn),
            };
        }
        if word == 0xd503201f || word & 0xfffffc1f == 0xd503241f {
            return Self::NoWrite;
        }
        // Remaining integer data-processing instructions overwrite Rd (31 is flags/ZR).
        if matches!((word >> 25) & 15, 8 | 9 | 5 | 13) {
            return Self::Write(rd);
        }
        Self::Unknown
    }

    fn relative(pc: u64, immediate: u32, bits: u32) -> u64 {
        let displacement = ((immediate as i32) << (32 - bits) >> (32 - bits)) as i64 * 4;
        pc.wrapping_add_signed(displacement)
    }

    pub(super) fn transfer(self, state: &mut [Origin; 31]) {
        let get = |register: u8| {
            state
                .get(register as usize)
                .copied()
                .unwrap_or(Origin::Constant(0))
        };
        let assigned = match self {
            Self::Move(dst, src, wide) => {
                let value = get(src);
                Some((
                    dst,
                    match (value, wide) {
                        (Origin::Constant(value), false) => Origin::Constant(value & 0xffffffff),
                        (_, false) => Origin::Unknown,
                        _ => value,
                    },
                ))
            }
            Self::Constant(dst, value) => Some((dst, Origin::Constant(value))),
            Self::Add(dst, src, delta, true) => Some((
                dst,
                match get(src) {
                    Origin::Argument(argument, offset) => offset
                        .checked_add(delta)
                        .map_or(Origin::Unknown, |offset| Origin::Argument(argument, offset)),
                    _ => Origin::Unknown,
                },
            )),
            Self::Add(dst, _, _, false) | Self::Write(dst) => Some((dst, Origin::Unknown)),
            Self::Load {
                target,
                base,
                offset,
                bytes,
            } => Some((
                target,
                match (get(base), bytes) {
                    (Origin::Argument(argument, base), 8) => {
                        Origin::Reference(argument, base + i64::from(offset))
                    }
                    _ => Origin::Unknown,
                },
            )),
            Self::Memory {
                first,
                second,
                writeback,
            } => {
                for register in [first, second, writeback].into_iter().flatten() {
                    if let Some(value) = state.get_mut(register as usize) {
                        *value = Origin::Unknown;
                    }
                }
                None
            }
            Self::Call(_) | Self::IndirectCall => {
                state[..19].fill(Origin::Unknown);
                state[30] = Origin::Unknown;
                None
            }
            Self::Unknown => {
                state.fill(Origin::Unknown);
                None
            }
            _ => None,
        };
        if let Some((dst, value)) = assigned {
            if let Some(destination) = state.get_mut(dst as usize) {
                *destination = value;
            }
        }
    }

    pub(super) fn boundary(self) -> bool {
        matches!(
            self,
            Self::Branch { .. }
                | Self::ZeroBranch { .. }
                | Self::Call(_)
                | Self::IndirectCall
                | Self::Return
                | Self::Stop
                | Self::Unknown
        )
    }
}
