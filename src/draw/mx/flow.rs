//! Control flow of a decoded stage: where each SYNC goes, and where the basic blocks start.
//!
//! The hardware keeps a stack of reconvergence points: SSY pushes one, SYNC pops it and jumps there. Effect shaders
//! use it in a fixed, nested way, so every SYNC can be given its target once, ahead of time, by walking every path
//! from the entry with the stack it would have. A program where one instruction is reached with two different stacks
//! is refused.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{anyhow, bail, Result};

use super::isa::{Instr, Op};

pub struct Flow {
    /// The resolved target of each SYNC reached, by instruction index.
    pub sync_target: HashMap<usize, u32>,
    /// Addresses that start a block: the entry and every jump target, ascending.
    pub leaders: BTreeSet<u32>,
    /// Address of the first instruction.
    pub entry: u32,
}

impl Flow {
    pub fn of(code: &[Instr]) -> Result<Flow> {
        let index: HashMap<u32, usize> = code.iter().enumerate().map(|(i, ins)| (ins.addr, i)).collect();
        let sync_target = reconvergence(code, &index)?;
        let entry = code.first().map_or(8, |ins| ins.addr);
        let mut leaders = BTreeSet::from([entry]);
        for (i, ins) in code.iter().enumerate() {
            match ins.op {
                Op::Bra { target } => {
                    leaders.insert(target);
                }
                // a SYNC no path reaches has no target; it starts nothing
                Op::Sync => {
                    leaders.insert(sync_target.get(&i).copied().unwrap_or(entry));
                }
                _ => {}
            }
        }
        if let Some(stray) = leaders.iter().find(|addr| !index.contains_key(addr)) {
            bail!("jump to {stray:#x}, which is not an instruction");
        }
        Ok(Flow { sync_target, leaders, entry })
    }

    /// Whether the program jumps at all (and so needs the dispatch loop).
    pub fn branches(&self) -> bool {
        self.leaders.len() > 1
    }
}

fn reconvergence(code: &[Instr], index: &HashMap<u32, usize>) -> Result<HashMap<usize, u32>> {
    let to = |addr: u32| index.get(&addr).copied().ok_or_else(|| anyhow!("jump to {addr:#x}, outside the program"));
    let mut seen: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
    let mut targets = HashMap::new();
    let mut pending: Vec<(usize, Vec<u32>)> = vec![(0, Vec::new())];
    while let Some((i, stack)) = pending.pop() {
        let Some(ins) = code.get(i) else { continue };
        if let Some(before) = seen.get(&i) {
            if *before != stack {
                bail!("{:#x} is reached with reconvergence stacks {before:x?} and {stack:x?}", ins.addr);
            }
            continue;
        }
        seen.insert(i, stack.clone());
        if ins.guard.never() {
            pending.push((i + 1, stack));
            continue;
        }
        // a guarded jump or exit may also fall through
        let may_fall = !ins.guard.always();
        match ins.op {
            Op::Exit => {
                if may_fall {
                    pending.push((i + 1, stack));
                }
            }
            Op::Bra { target } => {
                pending.push((to(target)?, stack.clone()));
                if may_fall {
                    pending.push((i + 1, stack));
                }
            }
            Op::Ssy { target } => {
                let mut pushed = stack;
                pushed.push(target);
                pending.push((i + 1, pushed));
            }
            Op::Sync => {
                let mut popped = stack.clone();
                let target = popped.pop().ok_or_else(|| anyhow!("SYNC at {:#x} with nothing to return to", ins.addr))?;
                targets.insert(i, target);
                pending.push((to(target)?, popped));
                if may_fall {
                    pending.push((i + 1, stack));
                }
            }
            _ => pending.push((i + 1, stack)),
        }
    }
    Ok(targets)
}
