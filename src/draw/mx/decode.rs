//! Maxwell (SM50) machine words to [`Instr`]s.
//!
//! Bit layouts are from the public Maxwell encodings (envytools' gm107 tables, Mesa's NAK sm50 backend). Only what
//! effect shaders use is accepted -- anything else is an error, so a stage that cannot be translated is known before
//! it is drawn. Code comes in bundles of four 64-bit words: one scheduling word, then three instructions.

use anyhow::{anyhow, bail, Result};

use super::isa::*;

/// One instruction word, read by bit position.
#[derive(Clone, Copy)]
struct Word(u64);

impl Word {
    fn field(self, at: u32, width: u32) -> u32 {
        ((self.0 >> at) & ((1u64 << width) - 1)) as u32
    }

    fn flag(self, at: u32) -> bool {
        self.field(at, 1) == 1
    }

    fn reg(self, at: u32) -> Reg {
        Reg(self.field(at, 8) as u8)
    }

    fn pred(self, at: u32, not_at: u32) -> Pred {
        Pred { index: self.field(at, 3) as u8, not: self.flag(not_at) }
    }

    /// The common modifier pair: negate at `neg`, absolute value at `abs`.
    fn mods(self, neg: u32, abs: u32) -> Mods {
        Mods { neg: self.flag(neg), abs: self.flag(abs) }
    }

    fn low32(self) -> Src {
        Src::Imm(self.field(20, 32))
    }

    /// The 20-bit immediate: 19 bits at 20, the sign at 56.
    fn imm20(self) -> u32 {
        self.field(20, 19) | (self.field(56, 1) << 19)
    }

    fn cbuf(self) -> Result<Src> {
        let offset = sign_extend(self.field(20, 14), 14) * 4;
        if offset < 0 {
            bail!("constant buffer offset {offset}");
        }
        Ok(Src::Cbuf { slot: self.field(34, 5) as u8, offset: offset as u32 })
    }

    /// The second operand of a register / constant / immediate form (top nibble 5 / 4 / other), the immediate as a
    /// float's top 20 bits or as a signed integer.
    fn b(self, float: bool) -> Result<Src> {
        Ok(match self.0 >> 60 {
            5 => Src::Reg(self.reg(20)),
            4 => self.cbuf()?,
            _ if float => Src::Imm(self.imm20() << 12),
            _ => Src::Imm(sign_extend(self.imm20(), 20) as u32),
        })
    }

    /// The instruction's own predicate operand.
    fn c39(self) -> Pred {
        self.pred(39, 42)
    }

    fn bool_op(self, at: u32) -> Result<BoolOp> {
        Ok(match self.field(at, 2) {
            0 => BoolOp::And,
            1 => BoolOp::Or,
            2 => BoolOp::Xor,
            v => bail!("boolean op {v}"),
        })
    }

    fn round(self, at: u32) -> Round {
        [Round::Nearest, Round::Down, Round::Up, Round::Zero][self.field(at, 2) as usize]
    }

    fn int_fmt(self, at: u32, sign_at: u32) -> Result<IntFmt> {
        Ok(match (self.field(at, 2), self.flag(sign_at)) {
            (0, false) => IntFmt::U8,
            (1, false) => IntFmt::U16,
            (2, false) => IntFmt::U32,
            (0, true) => IntFmt::S8,
            (1, true) => IntFmt::S16,
            (2, true) => IntFmt::S32,
            _ => bail!("64-bit integer"),
        })
    }

    fn float_fmt(self, at: u32) -> Result<FloatFmt> {
        Ok(match self.field(at, 2) {
            1 => FloatFmt::F16,
            2 => FloatFmt::F32,
            v => bail!("float format {v}"),
        })
    }

    /// Refuse a flag the translator does not model.
    fn refuse(self, at: u32, what: &str) -> Result<()> {
        if self.flag(at) {
            bail!("{what}");
        }
        Ok(())
    }

    /// Writing the condition code is not modelled (only I2I's, which CSETP reads).
    fn no_cc(self, at: u32) -> Result<()> {
        self.refuse(at, "writes the condition code")
    }

    /// Control instructions carry a condition code test; only "always" is supported.
    fn cc_always(self) -> Result<()> {
        match self.field(0, 5) {
            15 => Ok(()),
            t => bail!("condition code test {t}"),
        }
    }
}

fn sign_extend(v: u32, width: u32) -> i32 {
    ((v << (32 - width)) as i32) >> (32 - width)
}

/// Instruction kinds; the register, constant and immediate forms of one share a kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Sync,
    Depbar,
    St,
    Ld,
    Ldc,
    Kil,
    Exit,
    Ssy,
    Bra,
    Ipa,
    Tmml,
    Texs,
    I2i,
    Iadd3,
    I2f,
    F2i,
    F2f,
    Sel,
    Mov,
    Rro,
    Fmul,
    Fmnmx,
    Fadd,
    Lop,
    Iscadd,
    Iadd,
    Fsetp,
    Isetp,
    Iset,
    Xmad,
    Ffma,
    FfmaRc,
    Fset,
    Nop,
    Csetp,
    Psetp,
    Pset,
    Mufu,
    R2p,
    Shl,
    Shr,
    Fmul32i,
    Iadd32i,
    Fadd32i,
    Lop32i,
    Mov32i,
}

/// Opcode patterns of the top 16 bits as (value, mask, kind). Some patterns overlap, so the first match wins.
const OPCODES: &[(u16, u16, Kind)] = {
    use Kind::*;
    &[
        (0xf0f8, 0xfff8, Sync),
        (0xf0f0, 0xfff8, Depbar),
        (0xeff0, 0xfff8, St),
        (0xefd8, 0xfff8, Ld),
        (0xef90, 0xfff8, Ldc),
        (0xe330, 0xfff0, Kil),
        (0xe300, 0xfff0, Exit),
        (0xe290, 0xfff0, Ssy),
        (0xe240, 0xfff0, Bra),
        (0xe000, 0xff00, Ipa),
        (0xdf58, 0xfff8, Tmml),
        (0xd000, 0xf600, Texs),
        (0x5ce0, 0xfff8, I2i),
        (0x5cc0, 0xfff0, Iadd3),
        (0x5cb8, 0xfff8, I2f),
        (0x5cb0, 0xfff8, F2i),
        (0x5ca8, 0xfff8, F2f),
        (0x5ca0, 0xfff8, Sel),
        (0x5c98, 0xfff8, Mov),
        (0x5c90, 0xfff8, Rro),
        (0x5c68, 0xfff8, Fmul),
        (0x5c60, 0xfff8, Fmnmx),
        (0x5c58, 0xfff8, Fadd),
        (0x5c40, 0xfff8, Lop),
        (0x5c18, 0xfff8, Iscadd),
        (0x5c10, 0xfff8, Iadd),
        (0x5bb0, 0xfff0, Fsetp),
        (0x5b60, 0xfff0, Isetp),
        (0x5b50, 0xfff0, Iset),
        (0x5b00, 0xffc0, Xmad),
        (0x5980, 0xff80, Ffma),
        (0x5800, 0xff00, Fset),
        (0x5180, 0xff80, FfmaRc),
        (0x50b0, 0xfff8, Nop),
        (0x50a0, 0xfff8, Csetp),
        (0x5090, 0xfff8, Psetp),
        (0x5088, 0xfff8, Pset),
        (0x5080, 0xfff8, Mufu),
        (0x4cb8, 0xfff8, I2f),
        (0x4cb0, 0xfff8, F2i),
        (0x4ca0, 0xfff8, Sel),
        (0x4c98, 0xfff8, Mov),
        (0x4c68, 0xfff8, Fmul),
        (0x4c60, 0xfff8, Fmnmx),
        (0x4c58, 0xfff8, Fadd),
        (0x4c40, 0xfff8, Lop),
        (0x4bb0, 0xfff0, Fsetp),
        (0x4b60, 0xfff0, Isetp),
        (0x4b50, 0xfff0, Iset),
        (0x4980, 0xff80, Ffma),
        (0x4800, 0xfe00, Fset),
        (0x38f0, 0xfef8, R2p),
        (0x38a0, 0xfef8, Sel),
        (0x3868, 0xfef8, Fmul),
        (0x3860, 0xfef8, Fmnmx),
        (0x3858, 0xfef8, Fadd),
        (0x3848, 0xfef8, Shl),
        (0x3828, 0xfef8, Shr),
        (0x36b0, 0xfef0, Fsetp),
        (0x3660, 0xfef0, Isetp),
        (0x3650, 0xfef0, Iset),
        (0x3280, 0xfe80, Ffma),
        (0x3000, 0xfe00, Fset),
        (0x1e00, 0xff00, Fmul32i),
        (0x1c00, 0xfe80, Iadd32i),
        (0x0800, 0xfc00, Fadd32i),
        (0x0400, 0xfc00, Lop32i),
        (0x0100, 0xfff0, Mov32i),
    ]
};

fn kind(w: Word) -> Option<Kind> {
    let top = (w.0 >> 48) as u16;
    OPCODES.iter().find(|&&(value, mask, _)| top & mask == value).map(|&(_, _, k)| k)
}

/// A stage's instructions from the code after its program header.
pub fn decode_program(code: &[u8]) -> Result<Vec<Instr>> {
    code.chunks_exact(8)
        .enumerate()
        .filter(|(i, _)| i % 4 != 0)
        .map(|(i, bytes)| decode(u64::from_le_bytes(bytes.try_into().unwrap()), (i * 8) as u32))
        .collect()
}

/// The instruction `w` at byte address `addr`.
pub fn decode(w: u64, addr: u32) -> Result<Instr> {
    let w = Word(w);
    let kind = kind(w).ok_or_else(|| anyhow!("{addr:#06x}: {:016x} is not an instruction effect shaders use", w.0))?;
    let op = operation(w, kind, addr).map_err(|e| anyhow!("{addr:#06x}: {kind:?} {:016x}: {e}", w.0))?;
    Ok(Instr { addr, guard: w.pred(16, 19), op })
}

fn operation(w: Word, kind: Kind, addr: u32) -> Result<Op> {
    let d = w.reg(0);
    let a = w.reg(8);
    // FSETP / ISETP / PSETP / CSETP write two predicates
    let (p, q) = (w.field(3, 3) as u8, w.field(0, 3) as u8);
    Ok(match kind {
        Kind::Fadd => {
            w.no_cc(47)?;
            Op::Fadd { d, a, am: w.mods(48, 46), b: w.b(true)?, bm: w.mods(45, 49), sat: w.flag(50) }
        }
        Kind::Fadd32i => {
            w.no_cc(52)?;
            Op::Fadd { d, a, am: w.mods(56, 54), b: w.low32(), bm: w.mods(53, 57), sat: false }
        }
        Kind::Fmul => {
            w.no_cc(47)?;
            let scale = match w.field(41, 3) {
                0 => None,
                1 => Some(0.5),
                2 => Some(0.25),
                3 => Some(0.125),
                4 => Some(8.0),
                5 => Some(4.0),
                6 => Some(2.0),
                v => bail!("product scale {v}"),
            };
            Op::Fmul { d, a, b: w.b(true)?, neg_b: w.flag(48), sat: w.flag(50), scale, fmz: w.field(44, 2) == 2 }
        }
        Kind::Fmul32i => {
            w.no_cc(52)?;
            Op::Fmul { d, a, b: w.low32(), neg_b: false, sat: w.flag(55), scale: None, fmz: w.field(53, 2) == 2 }
        }
        Kind::Ffma | Kind::FfmaRc => {
            w.no_cc(47)?;
            // the register-constant form swaps where b and c come from
            let (b, c) = if kind == Kind::FfmaRc { (Src::Reg(w.reg(39)), w.cbuf()?) } else { (w.b(true)?, Src::Reg(w.reg(39))) };
            Op::Ffma { d, a, b, c, neg_b: w.flag(48), neg_c: w.flag(49), sat: w.flag(50), fmz: w.field(53, 2) == 2 }
        }
        Kind::Fmnmx => {
            w.no_cc(47)?;
            Op::Fmnmx { d, a, am: w.mods(48, 46), b: w.b(true)?, bm: w.mods(45, 49), min: w.c39() }
        }
        Kind::Mufu => {
            let func = match w.field(20, 4) {
                0 => Func::Cos,
                1 => Func::Sin,
                2 => Func::Ex2,
                3 => Func::Lg2,
                4 => Func::Rcp,
                5 => Func::Rsq,
                8 => Func::Sqrt,
                v => bail!("function {v}"),
            };
            Op::Mufu { d, a, m: w.mods(48, 46), func, sat: w.flag(50) }
        }
        Kind::Rro => Op::Rro { d, b: w.b(true)?, m: w.mods(45, 49) },
        Kind::Fset | Kind::Fsetp => {
            let dest = if kind == Kind::Fset {
                w.no_cc(47)?;
                CondDest::Reg { d, float_one: w.flag(52) }
            } else {
                CondDest::Preds { p, q }
            };
            // FSETP keeps its operand modifiers where FSET has its float-one flag and condition code
            let (am, bm) = if kind == Kind::Fset {
                (Mods { neg: w.flag(43), abs: w.flag(54) }, Mods { neg: w.flag(53), abs: w.flag(44) })
            } else {
                (Mods { neg: w.flag(43), abs: w.flag(7) }, Mods { neg: w.flag(6), abs: w.flag(44) })
            };
            let cmp = float_cmp(w.field(48, 4));
            let cond = Cond::Float { a, am, b: w.b(true)?, bm, cmp };
            Op::Set { cond, combine: w.bool_op(45)?, with: w.c39(), dest }
        }
        Kind::Iset | Kind::Isetp => {
            let dest = if kind == Kind::Iset {
                w.no_cc(47)?;
                CondDest::Reg { d, float_one: w.flag(44) }
            } else {
                CondDest::Preds { p, q }
            };
            w.refuse(43, "extended compare")?;
            let cond = Cond::Int { a, b: w.b(false)?, cmp: int_cmp(w.field(49, 3)), signed: w.flag(48) };
            Op::Set { cond, combine: w.bool_op(45)?, with: w.c39(), dest }
        }
        Kind::Pset | Kind::Psetp => {
            let dest = if kind == Kind::Pset {
                w.no_cc(47)?;
                CondDest::Reg { d, float_one: w.flag(44) }
            } else {
                CondDest::Preds { p, q }
            };
            let cond = Cond::Preds { a: w.pred(12, 15), b: w.pred(29, 32), op: w.bool_op(24)? };
            Op::Set { cond, combine: w.bool_op(45)?, with: w.c39(), dest }
        }
        Kind::Csetp => Op::Set { cond: Cond::Cc(w.field(8, 5) as u8), combine: w.bool_op(45)?, with: w.c39(), dest: CondDest::Preds { p, q } },
        Kind::Iadd => {
            w.no_cc(47)?;
            w.refuse(43, "extended add")?;
            w.refuse(50, "saturating add")?;
            Op::IntAdd { d, terms: [Some((Src::Reg(a), w.flag(49))), Some((w.b(false)?, w.flag(48))), None], shift: None }
        }
        Kind::Iadd32i => {
            w.no_cc(52)?;
            w.refuse(53, "extended add")?;
            w.refuse(54, "saturating add")?;
            Op::IntAdd { d, terms: [Some((Src::Reg(a), w.flag(56))), Some((w.low32(), false)), None], shift: None }
        }
        Kind::Iadd3 => {
            w.no_cc(47)?;
            if w.flag(48) || w.field(37, 2) != 0 || w.field(33, 4) != 0 {
                bail!("extended / shifted / half-word operands");
            }
            let terms = [Some((Src::Reg(a), w.flag(51))), Some((Src::Reg(w.reg(20)), w.flag(50))), Some((Src::Reg(w.reg(39)), w.flag(49)))];
            Op::IntAdd { d, terms, shift: None }
        }
        Kind::Iscadd => {
            w.no_cc(47)?;
            let terms = [Some((Src::Reg(a), w.flag(49))), Some((w.b(false)?, w.flag(48))), None];
            Op::IntAdd { d, terms, shift: Some(w.field(39, 5) as u8) }
        }
        Kind::Xmad => {
            w.no_cc(47)?;
            w.refuse(38, "extended multiply-add")?;
            let addend = match w.field(50, 3) {
                0 => XmadAddend::C,
                1 => XmadAddend::CLo,
                2 => XmadAddend::CHi,
                3 => XmadAddend::CSfu,
                4 => XmadAddend::CBcc,
                v => bail!("addend mode {v}"),
            };
            Op::Xmad {
                d,
                a,
                b: w.reg(20),
                c: w.reg(39),
                a_signed: w.flag(48),
                b_signed: w.flag(49),
                a_hi: w.flag(53),
                b_hi: w.flag(35),
                psl: w.flag(36),
                mrg: w.flag(37),
                addend,
            }
        }
        Kind::Lop => {
            w.no_cc(47)?;
            w.refuse(43, "extended logic op")?;
            let pout = match w.field(44, 2) {
                0 => None,
                1 => Some((w.field(48, 3) as u8, ZeroTest::Always)),
                2 => Some((w.field(48, 3) as u8, ZeroTest::Zero)),
                _ => Some((w.field(48, 3) as u8, ZeroTest::NonZero)),
            };
            Op::Lop { d, a, b: w.b(false)?, op: logic_op(w.field(41, 2)), inv_a: w.flag(39), inv_b: w.flag(40), pout }
        }
        Kind::Lop32i => {
            w.no_cc(52)?;
            w.refuse(57, "extended logic op")?;
            Op::Lop { d, a, b: w.low32(), op: logic_op(w.field(53, 2)), inv_a: w.flag(55), inv_b: w.flag(56), pout: None }
        }
        Kind::Shl => {
            w.no_cc(47)?;
            w.refuse(43, "extended shift")?;
            Op::Shift { d, a, b: w.b(false)?, left: true, signed: false, wrap: w.flag(39) }
        }
        Kind::Shr => {
            w.no_cc(47)?;
            w.refuse(44, "extended shift")?;
            w.refuse(40, "bit reverse")?;
            Op::Shift { d, a, b: w.b(false)?, left: false, signed: w.flag(48), wrap: w.flag(39) }
        }
        Kind::Sel => Op::Sel { d, a, b: w.b(false)?, c: w.c39() },
        Kind::Mov => {
            if w.field(39, 4) != 15 {
                bail!("partial lane mask");
            }
            Op::Mov { d, b: w.b(false)? }
        }
        Kind::Mov32i => {
            if w.field(12, 4) != 15 {
                bail!("partial lane mask");
            }
            Op::Mov { d, b: w.low32() }
        }
        Kind::I2f => {
            w.no_cc(47)?;
            let (b, to, from) = (w.b(false)?, w.float_fmt(8)?, w.int_fmt(10, 13)?);
            Op::I2f { d, b, to, from, byte: w.field(41, 2) as u8, m: w.mods(45, 49) }
        }
        Kind::F2i => {
            w.no_cc(47)?;
            let (b, to, from) = (w.b(true)?, w.int_fmt(8, 12)?, w.float_fmt(10)?);
            Op::F2i { d, b, to, from, round: w.round(39), m: w.mods(45, 49) }
        }
        Kind::F2f => {
            w.no_cc(47)?;
            // with bit 42 the value is rounded to an integer; without, only the default and toward-zero conversions
            let int_round = w.flag(42).then(|| w.round(39));
            if int_round.is_none() && !matches!(w.field(39, 2), 0 | 3) {
                bail!("directed rounding");
            }
            let (b, to, from) = (w.b(true)?, w.float_fmt(8)?, w.float_fmt(10)?);
            Op::F2f { d, b, to, from, int_round, high: w.flag(41), m: w.mods(45, 49), sat: w.flag(50) }
        }
        Kind::I2i => {
            let (b, to, from) = (w.b(false)?, w.int_fmt(8, 12)?, w.int_fmt(10, 13)?);
            Op::I2i { d, b, to, from, byte: w.field(41, 2) as u8, m: w.mods(45, 49), sat: w.flag(50), cc: w.flag(47) }
        }
        Kind::R2p => {
            if w.flag(40) || w.field(41, 2) != 0 {
                bail!("to the condition code / upper bytes");
            }
            let Src::Imm(mask) = w.b(false)? else { bail!("mask not immediate") };
            Op::R2p { a, mask }
        }
        Kind::Ld => {
            let count = w.field(47, 2) as u8 + 1;
            Op::Ald { d, offset: w.field(20, 10), index: a, vertex: w.reg(39), count, output: w.flag(32), patch: w.flag(31) }
        }
        Kind::St => {
            w.refuse(31, "patch store")?;
            Op::Ast { s: d, offset: w.field(20, 10), index: a, vertex: w.reg(39), count: w.field(47, 2) as u8 + 1 }
        }
        Kind::Ldc => {
            if w.field(44, 2) != 0 {
                bail!("indexed slot");
            }
            let count = match w.field(48, 3) {
                4 => 1,
                5 => 2,
                v => bail!("size {v}"),
            };
            Op::Ldc { d, slot: w.field(36, 5) as u8, index: a, offset: sign_extend(w.field(20, 16), 16), count }
        }
        Kind::Ipa => {
            w.refuse(38, "indexed attribute")?;
            if w.field(52, 2) != 0 {
                bail!("centroid / offset sampling");
            }
            let interp = [Interp::Pass, Interp::Multiply, Interp::Constant, Interp::Sc][w.field(54, 2) as usize];
            Op::Ipa { d, offset: w.field(28, 10), index: a, interp, mul: w.reg(20), sat: w.flag(51) }
        }
        Kind::Texs => {
            // before SM60, bit 59 tells TEXS from TLDS, and bit 57 marks TLD4S
            if !w.flag(59) || w.flag(57) {
                bail!("not a plain texture sample");
            }
            let shape = texs_shape(w.field(53, 4))?;
            let d1 = w.reg(28);
            let components = texs_components(w.field(50, 3), d1.is_zero())?;
            Op::Texs { d0: d, d1, a, b: w.reg(20), handle: w.field(36, 13) as u16, shape, components }
        }
        Kind::Tmml => {
            if w.flag(28) || w.field(29, 2) != 1 {
                bail!("not a 2D texture");
            }
            Op::Tmml { d, a, handle: w.field(36, 13) as u16, components: w.field(31, 4) as u8 }
        }
        Kind::Kil => {
            w.cc_always()?;
            Op::Kil
        }
        Kind::Exit => {
            w.cc_always()?;
            Op::Exit
        }
        Kind::Sync => {
            w.cc_always()?;
            Op::Sync
        }
        Kind::Bra | Kind::Ssy => {
            w.refuse(5, "target from a constant buffer")?;
            if kind == Kind::Bra {
                w.cc_always()?;
                w.refuse(6, "branch .lmt")?;
                w.refuse(7, "branch .u")?;
            }
            let mut target = (addr as i64 + 8 + sign_extend(w.field(20, 24), 24) as i64) as u32;
            // a bundle's scheduling word stands for the bundle: its first instruction
            if target % 32 == 0 {
                target += 8;
            }
            if kind == Kind::Bra {
                Op::Bra { target }
            } else {
                Op::Ssy { target }
            }
        }
        Kind::Nop | Kind::Depbar => Op::Nop,
    })
}

fn float_cmp(v: u32) -> FloatCmp {
    use FloatCmp::*;
    [F, Lt, Eq, Le, Gt, Ne, Ge, Num, Nan, Ltu, Equ, Leu, Gtu, Neu, Geu, T][v as usize]
}

fn int_cmp(v: u32) -> IntCmp {
    use IntCmp::*;
    [F, Lt, Eq, Le, Gt, Ne, Ge, T][v as usize]
}

fn logic_op(v: u32) -> LogicOp {
    [LogicOp::And, LogicOp::Or, LogicOp::Xor, LogicOp::PassB][v as usize]
}

fn texs_shape(v: u32) -> Result<TexShape> {
    use TexShape::*;
    const SHAPES: [TexShape; 14] = [Tex1DLz, Tex2D, Tex2DLz, Tex2DLl, Tex2DDc, Tex2DLlDc, Tex2DLzDc, Arr2D, Arr2DLz, Arr2DLzDc, Tex3D, Tex3DLz, Cube, CubeLl];
    SHAPES.get(v as usize).copied().ok_or_else(|| anyhow!("shape {v}"))
}

/// The components TEXS writes: with one destination pair the mask picks one or two, with two it picks three or four.
fn texs_components(mask: u32, one_pair: bool) -> Result<&'static [u8]> {
    const ONE_PAIR: [&[u8]; 8] = [&[0], &[1], &[2], &[3], &[0, 1], &[0, 3], &[1, 3], &[2, 3]];
    const TWO_PAIRS: [&[u8]; 5] = [&[0, 1, 2], &[0, 1, 3], &[0, 2, 3], &[1, 2, 3], &[0, 1, 2, 3]];
    if one_pair {
        Ok(ONE_PAIR[mask as usize])
    } else {
        TWO_PAIRS.get(mask as usize).copied().ok_or_else(|| anyhow!("write mask {mask}"))
    }
}
