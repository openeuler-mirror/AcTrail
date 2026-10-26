use std::collections::BTreeSet;

use super::instruction::{Instruction, Origin};
use super::{HandshakeLocation, Rejection};

#[derive(Clone, Copy, PartialEq, Eq)]
struct State([Origin; 31]);

impl State {
    fn entry() -> Self {
        Self(std::array::from_fn(|index| {
            Origin::Argument(index as u8, 0)
        }))
    }

    fn get(&self, register: u8) -> Origin {
        self.0
            .get(register as usize)
            .copied()
            .unwrap_or(Origin::Constant(0))
    }

    fn join(&mut self, incoming: Self) -> bool {
        let previous = *self;
        for (current, next) in self.0.iter_mut().zip(incoming.0) {
            if *current != next {
                *current = Origin::Unknown;
            }
        }
        previous != *self
    }
}

pub(super) struct PrefixEvidence {
    send_field: u16,
    branch: usize,
    exit: u64,
    reference: u8,
}

pub(super) struct FunctionAnalysis {
    start: u64,
    instructions: Vec<Instruction>,
}

impl FunctionAnalysis {
    pub(super) fn new(start: u64, words: Vec<u32>) -> Self {
        Self {
            start,
            instructions: words
                .into_iter()
                .enumerate()
                .map(|(index, word)| Instruction::decode(word, start + index as u64 * 4))
                .collect(),
        }
    }

    fn index(&self, address: u64) -> Option<usize> {
        let offset = address.checked_sub(self.start)?;
        if offset % 4 != 0 {
            return None;
        }
        let index = usize::try_from(offset / 4).ok()?;
        (index < self.instructions.len()).then_some(index)
    }

    fn consume(budget: &mut usize) -> Result<(), Rejection> {
        *budget = budget.checked_sub(1).ok_or(Rejection::DataflowBudget)?;
        Ok(())
    }

    pub(super) fn send_fields(&self, budget: &mut usize) -> Result<BTreeSet<u16>, Rejection> {
        let mut states = vec![None; self.instructions.len()];
        let mut queued = vec![false; self.instructions.len()];
        states[0] = Some(State::entry());
        let mut pending = vec![0];
        queued[0] = true;
        while let Some(index) = pending.pop() {
            Self::consume(budget)?;
            queued[index] = false;
            let instruction = self.instructions[index];
            let mut state = states[index].unwrap();
            instruction.transfer(&mut state.0);
            let mut successors = [None, None];
            match instruction {
                Instruction::Branch {
                    target,
                    conditional,
                } => {
                    successors[0] = self.index(target);
                    if conditional {
                        successors[1] = (index + 1 < states.len()).then_some(index + 1);
                    }
                }
                Instruction::ZeroBranch { target, .. } => {
                    successors[0] = self.index(target);
                    successors[1] = (index + 1 < states.len()).then_some(index + 1);
                }
                Instruction::Return | Instruction::Stop => {}
                _ => successors[0] = (index + 1 < states.len()).then_some(index + 1),
            }
            for next in successors.into_iter().flatten() {
                let changed = match &mut states[next] {
                    Some(current) => current.join(state),
                    empty => {
                        *empty = Some(state);
                        true
                    }
                };
                if changed && !queued[next] {
                    pending.push(next);
                    queued[next] = true;
                }
            }
        }
        let mut fields = BTreeSet::new();
        for (index, instruction) in self.instructions.iter().enumerate() {
            if let (
                Instruction::Load {
                    target,
                    base,
                    offset,
                    bytes: 1,
                },
                Some(state),
            ) = (instruction, states[index])
            {
                if state.get(*base) == Origin::Argument(0, 0)
                    && matches!(self.instructions.get(index + 1), Some(Instruction::CompareOne(register)) if register == target)
                    && matches!(
                        self.instructions.get(index + 2),
                        Some(Instruction::Branch {
                            conditional: true,
                            ..
                        })
                    )
                {
                    fields.insert(*offset);
                }
            }
        }
        if fields.is_empty() {
            return Err(Rejection::NoSendField);
        }
        Ok(fields)
    }

    pub(super) fn prefix(
        &self,
        fields: &BTreeSet<u16>,
        prefix: usize,
        budget: &mut usize,
    ) -> Result<Option<PrefixEvidence>, Rejection> {
        let mut state = State::entry();
        let mut writes = Vec::new();
        let mut nullable = None;
        let mut boundary = false;
        for (index, instruction) in self.instructions.iter().copied().take(prefix).enumerate() {
            Self::consume(budget)?;
            match instruction {
                Instruction::StoreByte {
                    value,
                    base,
                    offset,
                } if fields.contains(&offset)
                    && state.get(base) == Origin::Argument(0, 0)
                    && state.get(value) == Origin::Constant(1) =>
                {
                    writes.push(offset)
                }
                Instruction::ZeroBranch {
                    register,
                    target,
                    zero: true,
                } if state.get(register) == Origin::Reference(1, 0) => {
                    nullable = Some((index, target, register))
                }
                _ => {}
            }
            if instruction.boundary() {
                boundary = true;
                break;
            }
            instruction.transfer(&mut state.0);
        }
        if !boundary {
            return Err(Rejection::PrefixBudget);
        }
        let ([send_field], Some((branch, exit, reference))) = (writes.as_slice(), nullable) else {
            return Ok(None);
        };
        Ok(Some(PrefixEvidence {
            send_field: *send_field,
            branch,
            exit,
            reference,
        }))
    }

    pub(super) fn handshake(
        &self,
        prefix: PrefixEvidence,
        budget: &mut usize,
    ) -> Result<Option<HandshakeLocation>, Rejection> {
        let PrefixEvidence {
            send_field,
            branch,
            exit,
            reference,
        } = prefix;
        let Some(exit_index) = self.index(exit) else {
            return Ok(None);
        };
        let mut returns = false;
        for instruction in self.instructions[exit_index..].iter().copied() {
            Self::consume(budget)?;
            if matches!(instruction, Instruction::Return) {
                returns = true;
                break;
            }
            if instruction.boundary() {
                break;
            }
        }
        if !returns {
            return Ok(None);
        }
        let Some(Instruction::Load {
            target: length,
            base,
            bytes: 8,
            ..
        }) = self.instructions.get(branch + 1)
        else {
            return Ok(None);
        };
        if *base != reference
            || !matches!(self.instructions.get(branch + 2), Some(Instruction::ZeroBranch { register, target, zero: true }) if register == length && *target == exit)
        {
            return Ok(None);
        }
        let mut loops = false;
        let mut calls = false;
        for (index, instruction) in self
            .instructions
            .iter()
            .enumerate()
            .take(exit_index)
            .skip(branch + 3)
        {
            Self::consume(budget)?;
            let target = match instruction {
                Instruction::Branch { target, .. } | Instruction::ZeroBranch { target, .. } => {
                    Some(*target)
                }
                _ => None,
            };
            loops |= target.is_some_and(|target| {
                target > self.start + branch as u64 * 4 && target < self.start + index as u64 * 4
            });
            calls |= matches!(instruction, Instruction::Call(_));
        }
        Ok((loops && calls).then_some(HandshakeLocation {
            virtual_address: self.start,
            send_field,
            preceding_field: 0,
        }))
    }

    pub(super) fn preceding_fields(&self, budget: &mut usize) -> Result<BTreeSet<u16>, Rejection> {
        let mut state = State::entry();
        let mut stores = Vec::new();
        // The final instruction is the call being checked.
        for instruction in self
            .instructions
            .iter()
            .copied()
            .take(self.instructions.len() - 1)
        {
            Self::consume(budget)?;
            if instruction.boundary() {
                state = State::entry();
                stores.clear();
                continue;
            }
            if let Instruction::StoreByte {
                value,
                base,
                offset,
            } = instruction
            {
                if state.get(value) == Origin::Constant(1) {
                    if let Origin::Argument(origin, delta) = state.get(base) {
                        stores.push((origin, delta, offset));
                    }
                }
            }
            instruction.transfer(&mut state.0);
        }
        let mut fields = BTreeSet::new();
        if let Origin::Argument(argument, delta) = state.get(0) {
            for (origin, stored_delta, offset) in stores {
                if origin == argument {
                    if let Some(field) = stored_delta
                        .checked_add(i64::from(offset))
                        .and_then(|value| value.checked_sub(delta))
                        .and_then(|value| u16::try_from(value).ok())
                    {
                        fields.insert(field);
                    }
                }
            }
        }
        Ok(fields)
    }
}
