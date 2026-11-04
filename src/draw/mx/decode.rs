//! Maxwell (SM50) shader instructions, decoded: the input side of the effect shader translator.
//!
//! Encodings follow envytools' envydis/gm107.c and Mesa's NAK sm50 backend (both MIT). Only what the game's effect
//! shaders use is decoded -- 44 instructions, counted over every effect file in the game
//! -- and anything else is an error, so a shader the translator cannot
//! handle is known before it is drawn.
//!
//! The code is groups of four 64-bit words: a scheduling word (ignored: it only tells the hardware how long to wait)
//! and three instructions. Addresses are byte offsets from the first word after the program header, which is what
//! branch targets are relative to.

use anyhow::{bail, Result};

/// General purpose register; 255 reads as zero and discards writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Reg(pub u8);

impl Reg {
    pub const RZ: Reg = Reg(255);
    pub fn is_zero(self) -> bool {
        self.0 == 255
    }
    /// The register `n` above this one (vector operands use consecutive registers); RZ stays RZ.
    pub fn plus(self, n: u8) -> Reg {
        if self.is_zero() {
            self
        } else {
            Reg(self.0 + n)
        }
    }
}

/// Predicate register (0..6; 7 is PT, always true), optionally negated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pred {
    pub index: u8,
    pub not: bool,
}

impl Pred {
    pub const PT: Pred = Pred { index: 7, not: false };
    /// Always true (PT).
    pub fn is_true(self) -> bool {
        self.index == 7 && !self.not
    }
    /// Never true (!PT).
    pub fn is_false(self) -> bool {
        self.index == 7 && self.not
    }
}

/// The second source of most ALU instructions: a register, a constant buffer word, or an immediate (the raw 32 bits:
/// float immediates already widened to f32 bits, integer ones sign extended).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Src {
    Reg(Reg),
    /// `c[slot][offset]`, offset in bytes.
    Cbuf { slot: u8, offset: u32 },
    Imm(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatCmp {
    F,
    Lt,
    Eq,
    Le,
    Gt,
    Ne,
    Ge,
    Num,
    Nan,
    Ltu,
    Equ,
    Leu,
    Gtu,
    Neu,
    Geu,
    T,
}

impl FloatCmp {
    fn from(v: u32) -> FloatCmp {
        use FloatCmp::*;
        [F, Lt, Eq, Le, Gt, Ne, Ge, Num, Nan, Ltu, Equ, Leu, Gtu, Neu, Geu, T][v as usize & 15]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntCmp {
    F,
    Lt,
    Eq,
    Le,
    Gt,
    Ne,
    Ge,
    T,
}

impl IntCmp {
    fn from(v: u32) -> IntCmp {
        use IntCmp::*;
        [F, Lt, Eq, Le, Gt, Ne, Ge, T][v as usize & 7]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    And,
    Or,
    Xor,
}

impl BoolOp {
    fn from(v: u32) -> Result<BoolOp> {
        Ok(match v {
            0 => BoolOp::And,
            1 => BoolOp::Or,
            2 => BoolOp::Xor,
            _ => bail!("boolean op {v}"),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LopPred {
    True,
    Zero,
    NonZero,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicOp {
    And,
    Or,
    Xor,
    PassB,
}

/// Float rounding: nearest even (the default), down, up, toward zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Round {
    Rn,
    Rm,
    Rp,
    Rz,
}

impl Round {
    fn from(v: u32) -> Round {
        [Round::Rn, Round::Rm, Round::Rp, Round::Rz][v as usize & 3]
    }
}

/// Integer operand formats of the conversions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntFmt {
    U8,
    U16,
    U32,
    S8,
    S16,
    S32,
}

/// Float operand formats of the conversions. Effect shaders only convert to and from f32; f16 appears as the
/// packed half floats some vertex attributes are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatFmt {
    F16,
    F32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MufuOp {
    Cos,
    Sin,
    Ex2,
    Lg2,
    Rcp,
    Rsq,
    Sqrt,
}

/// FMUL's built-in scale of the product.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FmulScale {
    None,
    D2,
    D4,
    D8,
    M8,
    M4,
    M2,
}

/// XMAD's choice of addend: plain c, or c built from the operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XmadMode {
    C,
    CLo,
    CHi,
    CSfu,
    CBcc,
}

/// IPA's interpolation: perspective-correct with the 1/w in `mul` (`Multiply`), plain (`Pass`), flat (`Constant`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpaMode {
    Pass,
    Multiply,
    Constant,
    Sc,
}

/// TEXS's fixed set of sampling shapes (it is the short form of TEX: what it can do is the 4-bit field at 53).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexsKind {
    Tex1DLz,
    Tex2D,
    Tex2DLz,
    Tex2DLl,
    Tex2DDc,
    Tex2DLlDc,
    Tex2DLzDc,
    Arr2D,
    Arr2DLz,
    Arr2DLzDc,
    Tex3D,
    Tex3DLz,
    Cube,
    CubeLl,
}

/// Which component goes where in TEXS / TMML's writes: the components written, in order. The first two land in
/// `dst0` and the register after it, the rest in `dst1` and the one after it.
pub type Components = &'static [u8];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Fadd { d: Reg, a: Reg, b: Src, neg_a: bool, abs_a: bool, neg_b: bool, abs_b: bool, sat: bool, round: Round, ftz: bool },
    /// `fmz`: a zero factor makes the product zero, whatever the other one is (inf, NaN).
    Fmul { d: Reg, a: Reg, b: Src, neg_b: bool, sat: bool, round: Round, scale: FmulScale, fmz: bool },
    Ffma { d: Reg, a: Reg, b: Src, c: Src, neg_b: bool, neg_c: bool, sat: bool, round: Round, fmz: bool },
    Fmnmx { d: Reg, a: Reg, b: Src, neg_a: bool, abs_a: bool, neg_b: bool, abs_b: bool, min: Pred },
    Mufu { d: Reg, a: Reg, op: MufuOp, neg: bool, abs: bool, sat: bool },
    /// Range reduction before MUFU sin/cos/ex2: no-op in a translation that uses the real functions.
    Rro { d: Reg, b: Src, neg: bool, abs: bool },
    Fset { d: Reg, a: Reg, b: Src, cmp: FloatCmp, bop: BoolOp, c: Pred, neg_a: bool, abs_a: bool, neg_b: bool, abs_b: bool, bf: bool },
    Fsetp { p: u8, q: u8, a: Reg, b: Src, cmp: FloatCmp, bop: BoolOp, c: Pred, neg_a: bool, abs_a: bool, neg_b: bool, abs_b: bool },
    /// Predicate from the condition code: effect shaders only test it as "true" (CC is never written).
    Csetp { p: u8, q: u8, test: u8, bop: BoolOp, c: Pred },
    Psetp { p: u8, q: u8, a: Pred, b: Pred, c: Pred, op1: BoolOp, op2: BoolOp },
    Pset { d: Reg, a: Pred, b: Pred, c: Pred, op1: BoolOp, op2: BoolOp, bf: bool },
    Iset { d: Reg, a: Reg, b: Src, cmp: IntCmp, signed: bool, bop: BoolOp, c: Pred, bf: bool },
    Isetp { p: u8, q: u8, a: Reg, b: Src, cmp: IntCmp, signed: bool, bop: BoolOp, c: Pred },
    Iadd { d: Reg, a: Reg, b: Src, neg_a: bool, neg_b: bool },
    Iadd3 { d: Reg, a: Reg, b: Reg, c: Reg, neg_a: bool, neg_b: bool, neg_c: bool },
    Iscadd { d: Reg, a: Reg, b: Src, shift: u8, neg_a: bool, neg_b: bool },
    /// 16x16 multiply-add: `d = (a half * b half) [<< 16] + c-ish`, signs per operand.
    Xmad { d: Reg, a: Reg, b: Src, c: Src, a_signed: bool, b_signed: bool, a_hi: bool, b_hi: bool, psl: bool, mrg: bool, mode: XmadMode },
    /// `pout`: also set a predicate from the result (always / zero / non-zero).
    Lop { d: Reg, a: Reg, b: Src, op: LogicOp, inv_a: bool, inv_b: bool, pout: Option<(u8, LopPred)> },
    Shl { d: Reg, a: Reg, b: Src, wrap: bool },
    Shr { d: Reg, a: Reg, b: Src, signed: bool, wrap: bool },
    Sel { d: Reg, a: Reg, b: Src, c: Pred },
    /// `d = b` (MOV and MOV32I; the lane mask is always full in these shaders).
    Mov { d: Reg, b: Src },
    I2f { d: Reg, b: Src, dst: FloatFmt, src: IntFmt, byte: u8, neg: bool, abs: bool, round: Round },
    F2i { d: Reg, b: Src, dst: IntFmt, src: FloatFmt, round: Round, neg: bool, abs: bool },
    /// Float to float; `int_round` rounds to an integral value (F2F.FLOOR and friends), `half` picks the high half
    /// of a packed f16 source.
    F2f { d: Reg, b: Src, dst: FloatFmt, src: FloatFmt, int_round: Option<Round>, half: bool, neg: bool, abs: bool, sat: bool },
    /// `cc`: also sets the condition code's zero and sign flags from the result (CSETP tests them).
    I2i { d: Reg, b: Src, dst: IntFmt, src: IntFmt, byte: u8, neg: bool, abs: bool, sat: bool, cc: bool },
    /// Register to predicates: predicate i = bit i of `a & mask` (byte 0 only).
    R2p { a: Reg, mask: u32 },
    /// Attribute load: `count` words from attribute byte `offset` (+ `index` register), vertex `vertex`.
    Ald { d: Reg, offset: u32, index: Reg, vertex: Reg, count: u8, output: bool, patch: bool },
    /// Attribute store (vertex outputs).
    Ast { s: Reg, offset: u32, index: Reg, vertex: Reg, count: u8 },
    /// Constant load through a register: `count` words from `c[slot][index + offset]`.
    Ldc { d: Reg, slot: u8, index: Reg, offset: i32, count: u8 },
    /// Fragment input interpolation.
    Ipa { d: Reg, offset: u32, index: Reg, mode: IpaMode, mul: Reg, sat: bool },
    Texs { dst0: Reg, dst1: Reg, a: Reg, b: Reg, unit: u16, kind: TexsKind, components: Components },
    /// Level of detail query (textureQueryLod).
    Tmml { d: Reg, a: Reg, b: Reg, unit: u16, components: u8 },
    Kil,
    Exit,
    Bra { target: u32 },
    Ssy { target: u32 },
    Sync,
    Nop,
    Depbar,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instr {
    /// Byte address (from the first instruction word).
    pub addr: u32,
    /// Runs only when this holds.
    pub guard: Pred,
    pub op: Op,
}

fn bits(w: u64, at: u32, n: u32) -> u32 {
    ((w >> at) & ((1u64 << n) - 1)) as u32
}

fn bit(w: u64, at: u32) -> bool {
    (w >> at) & 1 != 0
}

fn reg(w: u64, at: u32) -> Reg {
    Reg(bits(w, at, 8) as u8)
}

fn pred(w: u64, at: u32, not_at: u32) -> Pred {
    Pred { index: bits(w, at, 3) as u8, not: bit(w, not_at) }
}

/// The 20-bit immediate: 19 bits at 20 and the top (sign) bit at 56.
fn imm20(w: u64) -> u32 {
    bits(w, 20, 19) | (bit(w, 56) as u32) << 19
}

fn sign_extend(v: u32, n: u32) -> i32 {
    ((v << (32 - n)) as i32) >> (32 - n)
}

/// The `b` operand by the encoding's form: register (0x5x), constant buffer (0x4x), immediate (0x3x).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    R,
    C,
    I,
}

fn form(w: u64) -> Form {
    match w >> 60 {
        5 => Form::R,
        4 => Form::C,
        _ => Form::I,
    }
}

fn cbuf(w: u64) -> Result<Src> {
    let offset = sign_extend(bits(w, 20, 14), 14) * 4;
    if offset < 0 {
        bail!("negative constant buffer offset {offset}");
    }
    Ok(Src::Cbuf { slot: bits(w, 34, 5) as u8, offset: offset as u32 })
}

/// `b` of an R/C/I instruction, the immediate read as a float.
fn src_f(w: u64) -> Result<Src> {
    Ok(match form(w) {
        Form::R => Src::Reg(reg(w, 20)),
        Form::C => cbuf(w)?,
        Form::I => Src::Imm(imm20(w) << 12),
    })
}

/// `b` of an R/C/I instruction, the immediate read as a signed integer.
fn src_i(w: u64) -> Result<Src> {
    Ok(match form(w) {
        Form::R => Src::Reg(reg(w, 20)),
        Form::C => cbuf(w)?,
        Form::I => Src::Imm(sign_extend(imm20(w), 20) as u32),
    })
}

fn int_fmt(size: u32, signed: bool) -> Result<IntFmt> {
    Ok(match (size, signed) {
        (0, false) => IntFmt::U8,
        (1, false) => IntFmt::U16,
        (2, false) => IntFmt::U32,
        (0, true) => IntFmt::S8,
        (1, true) => IntFmt::S16,
        (2, true) => IntFmt::S32,
        _ => bail!("64-bit integer conversion"),
    })
}

fn float_fmt(v: u32) -> Result<FloatFmt> {
    Ok(match v {
        1 => FloatFmt::F16,
        2 => FloatFmt::F32,
        _ => bail!("float format {v} (f64?)"),
    })
}

fn no_cc(w: u64, at: u32) -> Result<()> {
    if bit(w, at) {
        bail!("writes the condition code");
    }
    Ok(())
}

/// Condition-code test of control instructions: only "always" (15) is handled.
fn cc_always(w: u64, at: u32) -> Result<()> {
    let t = bits(w, at, 5);
    if t != 15 {
        bail!("condition code test {t}");
    }
    Ok(())
}

/// The instructions' (value, mask, name) in envydis' root table order (it takes the first match).
const TABLE: &[(u64, u64, &str)] = &[
    (0xf0f8, 0xfff8, "sync"),
    (0xf0f0, 0xfff8, "depbar"),
    (0xeff0, 0xfff8, "st"),
    (0xefd8, 0xfff8, "ld"),
    (0xef90, 0xfff8, "ldc"),
    (0xe330, 0xfff0, "kil"),
    (0xe300, 0xfff0, "exit"),
    (0xe290, 0xfff0, "ssy"),
    (0xe240, 0xfff0, "bra"),
    (0xe000, 0xff00, "ipa"),
    (0xdf58, 0xfff8, "tmml"),
    (0xd000, 0xf600, "texs"),
    (0x5ce0, 0xfff8, "i2i"),
    (0x5cc0, 0xfff0, "iadd3"),
    (0x5cb8, 0xfff8, "i2f"),
    (0x5cb0, 0xfff8, "f2i"),
    (0x5ca8, 0xfff8, "f2f"),
    (0x5ca0, 0xfff8, "sel"),
    (0x5c98, 0xfff8, "mov"),
    (0x5c90, 0xfff8, "rro"),
    (0x5c68, 0xfff8, "fmul"),
    (0x5c60, 0xfff8, "fmnmx"),
    (0x5c58, 0xfff8, "fadd"),
    (0x5c40, 0xfff8, "lop"),
    (0x5c18, 0xfff8, "iscadd"),
    (0x5c10, 0xfff8, "iadd"),
    (0x5bb0, 0xfff0, "fsetp"),
    (0x5b60, 0xfff0, "isetp"),
    (0x5b50, 0xfff0, "iset"),
    (0x5b00, 0xffc0, "xmad"),
    (0x5980, 0xff80, "ffma"),
    (0x5800, 0xff00, "fset"),
    (0x5180, 0xff80, "ffma_rc"),
    (0x50b0, 0xfff8, "nop"),
    (0x50a0, 0xfff8, "csetp"),
    (0x5090, 0xfff8, "psetp"),
    (0x5088, 0xfff8, "pset"),
    (0x5080, 0xfff8, "mufu"),
    (0x4cb8, 0xfff8, "i2f"),
    (0x4cb0, 0xfff8, "f2i"),
    (0x4ca0, 0xfff8, "sel"),
    (0x4c98, 0xfff8, "mov"),
    (0x4c68, 0xfff8, "fmul"),
    (0x4c60, 0xfff8, "fmnmx"),
    (0x4c58, 0xfff8, "fadd"),
    (0x4c40, 0xfff8, "lop"),
    (0x4bb0, 0xfff0, "fsetp"),
    (0x4b60, 0xfff0, "isetp"),
    (0x4b50, 0xfff0, "iset"),
    (0x4980, 0xff80, "ffma"),
    (0x4800, 0xfe00, "fset"),
    (0x38f0, 0xfef8, "r2p"),
    (0x38a0, 0xfef8, "sel"),
    (0x3868, 0xfef8, "fmul"),
    (0x3860, 0xfef8, "fmnmx"),
    (0x3858, 0xfef8, "fadd"),
    (0x3848, 0xfef8, "shl"),
    (0x3828, 0xfef8, "shr"),
    (0x36b0, 0xfef0, "fsetp"),
    (0x3660, 0xfef0, "isetp"),
    (0x3650, 0xfef0, "iset"),
    (0x3280, 0xfe80, "ffma"),
    (0x3000, 0xfe00, "fset"),
    (0x1e00, 0xff00, "fmul32i"),
    (0x1c00, 0xfe80, "iadd32i"),
    (0x0800, 0xfc00, "fadd32i"),
    (0x0400, 0xfc00, "lop32i"),
    (0x0100, 0xfff0, "mov32i"),
];

/// The instruction's name in the table (the R/C/I forms share one), or None for anything outside the subset.
pub fn name(w: u64) -> Option<&'static str> {
    let top = (w >> 48) as u16 as u64;
    TABLE.iter().find(|(v, m, _)| top & m == *v).map(|(_, _, n)| *n)
}

/// One instruction word at `addr`.
pub fn decode(w: u64, addr: u32) -> Result<Instr> {
    let Some(name) = name(w) else {
        bail!("{addr:#06x}: instruction {w:016x} is not one effect shaders use");
    };
    let guard = Pred { index: bits(w, 16, 3) as u8, not: bit(w, 19) };
    let op = decode_op(w, addr, name).map_err(|e| anyhow::anyhow!("{addr:#06x}: {name} {w:016x}: {e}"))?;
    Ok(Instr { addr, guard, op })
}

fn decode_op(w: u64, addr: u32, name: &str) -> Result<Op> {
    let d = reg(w, 0);
    let a = reg(w, 8);
    let c39 = pred(w, 39, 42);
    Ok(match name {
        "fadd" => {
            no_cc(w, 47)?;
            Op::Fadd {
                d,
                a,
                b: src_f(w)?,
                neg_a: bit(w, 48),
                abs_a: bit(w, 46),
                neg_b: bit(w, 45),
                abs_b: bit(w, 49),
                sat: bit(w, 50),
                round: Round::from(bits(w, 39, 2)),
                ftz: bit(w, 44),
            }
        }
        "fadd32i" => {
            no_cc(w, 52)?;
            Op::Fadd {
                d,
                a,
                b: Src::Imm(bits(w, 20, 32)),
                neg_a: bit(w, 56),
                abs_a: bit(w, 54),
                neg_b: bit(w, 53),
                abs_b: bit(w, 57),
                sat: false,
                round: Round::Rn,
                ftz: bit(w, 55),
            }
        }
        "fmul" => {
            no_cc(w, 47)?;
            let scale = match bits(w, 41, 3) {
                0 => FmulScale::None,
                1 => FmulScale::D2,
                2 => FmulScale::D4,
                3 => FmulScale::D8,
                4 => FmulScale::M8,
                5 => FmulScale::M4,
                6 => FmulScale::M2,
                v => bail!("fmul scale {v}"),
            };
            Op::Fmul { d, a, b: src_f(w)?, neg_b: bit(w, 48), sat: bit(w, 50), round: Round::from(bits(w, 39, 2)), scale, fmz: bits(w, 44, 2) == 2 }
        }
        "fmul32i" => {
            no_cc(w, 52)?;
            Op::Fmul {
                d,
                a,
                b: Src::Imm(bits(w, 20, 32)),
                neg_b: false,
                sat: bit(w, 55),
                round: Round::Rn,
                scale: FmulScale::None,
                fmz: bits(w, 53, 2) == 2,
            }
        }
        "ffma" | "ffma_rc" => {
            no_cc(w, 47)?;
            let (b, c) = match (name, form(w)) {
                ("ffma_rc", _) => (Src::Reg(reg(w, 39)), cbuf(w)?),
                (_, Form::R) => (Src::Reg(reg(w, 20)), Src::Reg(reg(w, 39))),
                (_, Form::C) => (cbuf(w)?, Src::Reg(reg(w, 39))),
                (_, Form::I) => (Src::Imm(imm20(w) << 12), Src::Reg(reg(w, 39))),
            };
            Op::Ffma {
                d,
                a,
                b,
                c,
                neg_b: bit(w, 48),
                neg_c: bit(w, 49),
                sat: bit(w, 50),
                round: Round::from(bits(w, 51, 2)),
                fmz: bits(w, 53, 2) == 2,
            }
        }
        "fmnmx" => {
            no_cc(w, 47)?;
            Op::Fmnmx {
                d,
                a,
                b: src_f(w)?,
                neg_a: bit(w, 48),
                abs_a: bit(w, 46),
                neg_b: bit(w, 45),
                abs_b: bit(w, 49),
                min: c39,
            }
        }
        "mufu" => {
            let op = match bits(w, 20, 4) {
                0 => MufuOp::Cos,
                1 => MufuOp::Sin,
                2 => MufuOp::Ex2,
                3 => MufuOp::Lg2,
                4 => MufuOp::Rcp,
                5 => MufuOp::Rsq,
                8 => MufuOp::Sqrt,
                v => bail!("mufu function {v}"),
            };
            Op::Mufu { d, a, op, neg: bit(w, 48), abs: bit(w, 46), sat: bit(w, 50) }
        }
        "rro" => Op::Rro { d, b: src_f(w)?, neg: bit(w, 45), abs: bit(w, 49) },
        "fset" => {
            no_cc(w, 47)?;
            Op::Fset {
                d,
                a,
                b: src_f(w)?,
                cmp: FloatCmp::from(bits(w, 48, 4)),
                bop: BoolOp::from(bits(w, 45, 2))?,
                c: c39,
                neg_a: bit(w, 43),
                abs_a: bit(w, 54),
                neg_b: bit(w, 53),
                abs_b: bit(w, 44),
                bf: bit(w, 52),
            }
        }
        "fsetp" => Op::Fsetp {
            p: bits(w, 3, 3) as u8,
            q: bits(w, 0, 3) as u8,
            a,
            b: src_f(w)?,
            cmp: FloatCmp::from(bits(w, 48, 4)),
            bop: BoolOp::from(bits(w, 45, 2))?,
            c: c39,
            neg_a: bit(w, 43),
            abs_a: bit(w, 7),
            neg_b: bit(w, 6),
            abs_b: bit(w, 44),
        },
        "csetp" => Op::Csetp {
            p: bits(w, 3, 3) as u8,
            q: bits(w, 0, 3) as u8,
            test: bits(w, 8, 5) as u8,
            bop: BoolOp::from(bits(w, 45, 2))?,
            c: c39,
        },
        "psetp" => Op::Psetp {
            p: bits(w, 3, 3) as u8,
            q: bits(w, 0, 3) as u8,
            a: pred(w, 12, 15),
            b: pred(w, 29, 32),
            c: c39,
            op1: BoolOp::from(bits(w, 24, 2))?,
            op2: BoolOp::from(bits(w, 45, 2))?,
        },
        "pset" => {
            no_cc(w, 47)?;
            Op::Pset {
                d,
                a: pred(w, 12, 15),
                b: pred(w, 29, 32),
                c: c39,
                op1: BoolOp::from(bits(w, 24, 2))?,
                op2: BoolOp::from(bits(w, 45, 2))?,
                bf: bit(w, 44),
            }
        }
        "iset" => {
            no_cc(w, 47)?;
            if bit(w, 43) {
                bail!("extended (.x) compare");
            }
            Op::Iset {
                d,
                a,
                b: src_i(w)?,
                cmp: IntCmp::from(bits(w, 49, 3)),
                signed: bit(w, 48),
                bop: BoolOp::from(bits(w, 45, 2))?,
                c: c39,
                bf: bit(w, 44),
            }
        }
        "isetp" => {
            if bit(w, 43) {
                bail!("extended (.x) compare");
            }
            Op::Isetp {
                p: bits(w, 3, 3) as u8,
                q: bits(w, 0, 3) as u8,
                a,
                b: src_i(w)?,
                cmp: IntCmp::from(bits(w, 49, 3)),
                signed: bit(w, 48),
                bop: BoolOp::from(bits(w, 45, 2))?,
                c: c39,
            }
        }
        "iadd" => {
            no_cc(w, 47)?;
            if bit(w, 43) || bit(w, 50) {
                bail!("extended or saturating add");
            }
            Op::Iadd { d, a, b: src_i(w)?, neg_a: bit(w, 49), neg_b: bit(w, 48) }
        }
        "iadd32i" => {
            no_cc(w, 52)?;
            if bit(w, 53) || bit(w, 54) {
                bail!("extended or saturating add");
            }
            Op::Iadd { d, a, b: Src::Imm(bits(w, 20, 32)), neg_a: bit(w, 56), neg_b: false }
        }
        "iadd3" => {
            no_cc(w, 47)?;
            if bit(w, 48) || bits(w, 37, 2) != 0 || bits(w, 33, 4) != 0 {
                bail!("iadd3 extended / shifted / half operands");
            }
            Op::Iadd3 { d, a, b: reg(w, 20), c: reg(w, 39), neg_a: bit(w, 51), neg_b: bit(w, 50), neg_c: bit(w, 49) }
        }
        "iscadd" => {
            no_cc(w, 47)?;
            Op::Iscadd { d, a, b: src_i(w)?, shift: bits(w, 39, 5) as u8, neg_a: bit(w, 49), neg_b: bit(w, 48) }
        }
        "xmad" => {
            no_cc(w, 47)?;
            if bit(w, 38) {
                bail!("extended xmad");
            }
            let mode = match bits(w, 50, 3) {
                0 => XmadMode::C,
                1 => XmadMode::CLo,
                2 => XmadMode::CHi,
                3 => XmadMode::CSfu,
                4 => XmadMode::CBcc,
                v => bail!("xmad mode {v}"),
            };
            Op::Xmad {
                d,
                a,
                b: Src::Reg(reg(w, 20)),
                c: Src::Reg(reg(w, 39)),
                a_signed: bit(w, 48),
                b_signed: bit(w, 49),
                a_hi: bit(w, 53),
                b_hi: bit(w, 35),
                psl: bit(w, 36),
                mrg: bit(w, 37),
                mode,
            }
        }
        "lop" => {
            no_cc(w, 47)?;
            if bit(w, 43) {
                bail!("lop extended");
            }
            let pout = match bits(w, 44, 2) {
                0 => None,
                m => Some((bits(w, 48, 3) as u8, [LopPred::True, LopPred::True, LopPred::Zero, LopPred::NonZero][m as usize])),
            };
            let op = [LogicOp::And, LogicOp::Or, LogicOp::Xor, LogicOp::PassB][bits(w, 41, 2) as usize];
            Op::Lop { d, a, b: src_i(w)?, op, inv_a: bit(w, 39), inv_b: bit(w, 40), pout }
        }
        "lop32i" => {
            no_cc(w, 52)?;
            if bit(w, 57) {
                bail!("lop32i extended");
            }
            let op = [LogicOp::And, LogicOp::Or, LogicOp::Xor, LogicOp::PassB][bits(w, 53, 2) as usize];
            Op::Lop { d, a, b: Src::Imm(bits(w, 20, 32)), op, inv_a: bit(w, 55), inv_b: bit(w, 56), pout: None }
        }
        "shl" => {
            no_cc(w, 47)?;
            if bit(w, 43) {
                bail!("shl extended");
            }
            Op::Shl { d, a, b: src_i(w)?, wrap: bit(w, 39) }
        }
        "shr" => {
            no_cc(w, 47)?;
            if bit(w, 44) || bit(w, 40) {
                bail!("shr extended / bit reverse");
            }
            Op::Shr { d, a, b: src_i(w)?, signed: bit(w, 48), wrap: bit(w, 39) }
        }
        "sel" => Op::Sel { d, a, b: src_i(w)?, c: c39 },
        "mov" => {
            if bits(w, 39, 4) != 15 {
                bail!("mov lane mask {:x}", bits(w, 39, 4));
            }
            Op::Mov { d, b: src_i(w)? }
        }
        "mov32i" => {
            if bits(w, 12, 4) != 15 {
                bail!("mov32i lane mask {:x}", bits(w, 12, 4));
            }
            Op::Mov { d, b: Src::Imm(bits(w, 20, 32)) }
        }
        "i2f" => {
            no_cc(w, 47)?;
            Op::I2f {
                d,
                b: src_i(w)?,
                dst: float_fmt(bits(w, 8, 2))?,
                src: int_fmt(bits(w, 10, 2), bit(w, 13))?,
                byte: bits(w, 41, 2) as u8,
                neg: bit(w, 45),
                abs: bit(w, 49),
                round: Round::from(bits(w, 39, 2)),
            }
        }
        "f2i" => {
            no_cc(w, 47)?;
            Op::F2i {
                d,
                b: src_f(w)?,
                dst: int_fmt(bits(w, 8, 2), bit(w, 12))?,
                src: float_fmt(bits(w, 10, 2))?,
                round: Round::from(bits(w, 39, 2)),
                neg: bit(w, 45),
                abs: bit(w, 49),
            }
        }
        "f2f" => {
            no_cc(w, 47)?;
            // bits 39/40 round mode, 42 = round to an integral value
            let int_round = bit(w, 42).then(|| Round::from(bits(w, 39, 2)));
            if !bit(w, 42) && bits(w, 39, 2) != 0 && bits(w, 39, 2) != 3 {
                bail!("f2f directed rounding {}", bits(w, 39, 2));
            }
            Op::F2f {
                d,
                b: src_f(w)?,
                dst: float_fmt(bits(w, 8, 2))?,
                src: float_fmt(bits(w, 10, 2))?,
                int_round,
                half: bit(w, 41),
                neg: bit(w, 45),
                abs: bit(w, 49),
                sat: bit(w, 50),
            }
        }
        "i2i" => {
            Op::I2i {
                d,
                b: src_i(w)?,
                dst: int_fmt(bits(w, 8, 2), bit(w, 12))?,
                src: int_fmt(bits(w, 10, 2), bit(w, 13))?,
                byte: bits(w, 41, 2) as u8,
                neg: bit(w, 45),
                abs: bit(w, 49),
                sat: bit(w, 50),
                cc: bit(w, 47),
            }
        }
        "r2p" => {
            if bit(w, 40) || bits(w, 41, 2) != 0 {
                bail!("r2p to the condition code / upper byte");
            }
            let Src::Imm(mask) = src_i(w)? else { bail!("r2p mask from a register") };
            Op::R2p { a, mask }
        }
        "ld" | "st" => {
            let count = bits(w, 47, 2) as u8 + 1;
            let offset = bits(w, 20, 10);
            if name == "ld" {
                Op::Ald { d, offset, index: a, vertex: reg(w, 39), count, output: bit(w, 32), patch: bit(w, 31) }
            } else {
                if bit(w, 31) {
                    bail!("patch attribute store");
                }
                Op::Ast { s: d, offset, index: a, vertex: reg(w, 39), count }
            }
        }
        "ldc" => {
            if bits(w, 44, 2) != 0 {
                bail!("ldc indexed-slot mode {}", bits(w, 44, 2));
            }
            let count = match bits(w, 48, 3) {
                4 => 1,
                5 => 2,
                v => bail!("ldc size {v} (sub-word)"),
            };
            Op::Ldc { d, slot: bits(w, 36, 5) as u8, index: a, offset: sign_extend(bits(w, 20, 16), 16), count }
        }
        "ipa" => {
            if bit(w, 38) {
                bail!("ipa with an attribute index register");
            }
            if bits(w, 52, 2) != 0 {
                bail!("ipa centroid / offset sampling");
            }
            let mode = [IpaMode::Pass, IpaMode::Multiply, IpaMode::Constant, IpaMode::Sc][bits(w, 54, 2) as usize];
            Op::Ipa { d, offset: bits(w, 28, 10), index: a, mode, mul: reg(w, 20), sat: bit(w, 51) }
        }
        "texs" => {
            // bit 59 is part of the opcode before SM60 (TEXS is 0xd8.., TLDS 0xda..); SM60 reuses it for f16
            if !bit(w, 59) || bit(w, 57) {
                bail!("not texs (tlds / tld4s?)");
            }
            use TexsKind::*;
            let kind = match bits(w, 53, 4) {
                0 => Tex1DLz,
                1 => Tex2D,
                2 => Tex2DLz,
                3 => Tex2DLl,
                4 => Tex2DDc,
                5 => Tex2DLlDc,
                6 => Tex2DLzDc,
                7 => Arr2D,
                8 => Arr2DLz,
                9 => Arr2DLzDc,
                10 => Tex3D,
                11 => Tex3DLz,
                12 => Cube,
                13 => CubeLl,
                v => bail!("texs kind {v}"),
            };
            let dst1 = reg(w, 28);
            let mask = bits(w, 50, 3);
            // one destination register pair: the two-component set; two: the three/four-component set
            let components: Components = if dst1.is_zero() {
                match mask {
                    0 => &[0],
                    1 => &[1],
                    2 => &[2],
                    3 => &[3],
                    4 => &[0, 1],
                    5 => &[0, 3],
                    6 => &[1, 3],
                    _ => &[2, 3],
                }
            } else {
                match mask {
                    0 => &[0, 1, 2],
                    1 => &[0, 1, 3],
                    2 => &[0, 2, 3],
                    3 => &[1, 2, 3],
                    4 => &[0, 1, 2, 3],
                    v => bail!("texs mask {v}"),
                }
            };
            Op::Texs { dst0: d, dst1, a, b: reg(w, 20), unit: bits(w, 36, 13) as u16, kind, components }
        }
        "tmml" => {
            if bit(w, 28) || bits(w, 29, 2) != 1 {
                bail!("tmml on a non-2D texture");
            }
            Op::Tmml { d, a, b: reg(w, 20), unit: bits(w, 36, 13) as u16, components: bits(w, 31, 4) as u8 }
        }
        "kil" => {
            cc_always(w, 0)?;
            Op::Kil
        }
        "exit" => {
            cc_always(w, 0)?;
            Op::Exit
        }
        "sync" => {
            cc_always(w, 0)?;
            Op::Sync
        }
        "bra" | "ssy" => {
            if bit(w, 5) {
                bail!("{name} to a constant buffer address");
            }
            if name == "bra" {
                cc_always(w, 0)?;
                if bit(w, 6) || bit(w, 7) {
                    bail!("bra .lmt / .u");
                }
            }
            let mut target = (addr as i64 + 8 + sign_extend(bits(w, 20, 24), 24) as i64) as u32;
            // a target on a scheduling word (bundles are 32 bytes) means the bundle's first instruction
            if target % 32 == 0 {
                target += 8;
            }
            if name == "bra" {
                Op::Bra { target }
            } else {
                Op::Ssy { target }
            }
        }
        "nop" => Op::Nop,
        "depbar" => Op::Depbar,
        _ => unreachable!("{name}"),
    })
}

/// A stage's instructions, from the code after the 0x50-byte program header (`code` = the NVN code section from its
/// header at +0x30, `len` = the control section's code length, header included).
pub fn decode_program(code: &[u8]) -> Result<Vec<Instr>> {
    let mut out = Vec::with_capacity(code.len() / 8);
    for (i, chunk) in code.chunks_exact(8).enumerate() {
        if i % 4 == 0 {
            continue; // scheduling word
        }
        let w = u64::from_le_bytes(chunk.try_into().unwrap());
        out.push(decode(w, (i * 8) as u32)?);
    }
    Ok(out)
}
