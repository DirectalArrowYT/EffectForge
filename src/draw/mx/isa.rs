//! What the translator knows of a Maxwell instruction once it is decoded: only the operations the game's effect
//! shaders use, each with the operands and modifiers that change what it computes. Scheduling, rounding modes the
//! WGSL cannot express, and reuse hints are not kept.

/// A general purpose register. Register 255 (RZ) reads as zero and drops what is written to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Reg(pub u8);

impl Reg {
    pub const RZ: Reg = Reg(255);

    pub fn is_zero(self) -> bool {
        self == Self::RZ
    }

    /// The `k`th register of a vector operand starting here (vectors use consecutive registers); RZ stays RZ.
    pub fn nth(self, k: u8) -> Reg {
        if self.is_zero() {
            self
        } else {
            Reg(self.0 + k)
        }
    }
}

/// A predicate register, maybe inverted. Index 7 is PT, which is always true.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pred {
    pub index: u8,
    pub not: bool,
}

impl Pred {
    pub const PT: u8 = 7;

    pub fn always(self) -> bool {
        self.index == Self::PT && !self.not
    }

    pub fn never(self) -> bool {
        self.index == Self::PT && self.not
    }
}

/// A source operand that may be more than a register.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Src {
    Reg(Reg),
    /// Word `offset / 4` of constant buffer `slot` (`offset` in bytes).
    Cbuf { slot: u8, offset: u32 },
    /// The operand's 32 bits: a float immediate already widened to f32 bits, an integer one sign extended.
    Imm(u32),
}

/// The negate and absolute value modifiers of a float operand (absolute value first).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub neg: bool,
    pub abs: bool,
}

/// Float comparisons, in their encoding order: ordered ones, then the unordered (`u`) ones, which also hold when an
/// operand is NaN.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    And,
    Or,
    Xor,
}

/// What a comparison-type instruction tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cond {
    Float { a: Reg, am: Mods, b: Src, bm: Mods, cmp: FloatCmp },
    Int { a: Reg, b: Src, cmp: IntCmp, signed: bool },
    /// Two predicates combined.
    Preds { a: Pred, b: Pred, op: BoolOp },
    /// A condition code test (the encoding's 5-bit test number).
    Cc(u8),
}

/// Where a comparison's result goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CondDest {
    /// A register, as all ones (or 1.0 when `float_one`) when true and 0 when false.
    Reg { d: Reg, float_one: bool },
    /// Predicate `p` gets the result, `q` its inverse (both combined with the instruction's predicate).
    Preds { p: u8, q: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicOp {
    And,
    Or,
    Xor,
    PassB,
}

/// A predicate a logic op can also write from its result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZeroTest {
    Always,
    Zero,
    NonZero,
}

/// Float rounding: to nearest even, down, up, toward zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Round {
    Nearest,
    Down,
    Up,
    Zero,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntFmt {
    U8,
    U16,
    U32,
    S8,
    S16,
    S32,
}

impl IntFmt {
    pub fn unsigned(self) -> bool {
        matches!(self, IntFmt::U8 | IntFmt::U16 | IntFmt::U32)
    }
}

/// Conversions only meet f32, and f16 as the packed halves some vertex attributes are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatFmt {
    F16,
    F32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Cos,
    Sin,
    Ex2,
    Lg2,
    Rcp,
    Rsq,
    Sqrt,
}

/// What XMAD adds to its 16x16 product.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XmadAddend {
    C,
    CLo,
    CHi,
    CSfu,
    CBcc,
}

/// How IPA interpolates: plain, multiplied by its `mul` register (perspective correction), flat, or `.sc`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Pass,
    Multiply,
    Constant,
    Sc,
}

/// The sampling shapes the short texture instruction (TEXS) offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexShape {
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Fadd { d: Reg, a: Reg, am: Mods, b: Src, bm: Mods, sat: bool },
    /// `scale`: the product's built-in factor; `fmz`: a zero factor gives zero whatever the other is.
    Fmul { d: Reg, a: Reg, b: Src, neg_b: bool, sat: bool, scale: Option<f32>, fmz: bool },
    Ffma { d: Reg, a: Reg, b: Src, c: Src, neg_b: bool, neg_c: bool, sat: bool, fmz: bool },
    /// Min where `min` holds, max where it does not.
    Fmnmx { d: Reg, a: Reg, am: Mods, b: Src, bm: Mods, min: Pred },
    Mufu { d: Reg, a: Reg, m: Mods, func: Func, sat: bool },
    /// The range reduction the hardware wants before MUFU; WGSL's functions take the plain value.
    Rro { d: Reg, b: Src, m: Mods },
    /// Every compare-and-set: FSET(P), ISET(P), PSET(P), CSETP. The test's result is combined with `with` by `combine`.
    Set { cond: Cond, combine: BoolOp, with: Pred, dest: CondDest },
    /// IADD, IADD32I, IADD3 and ISCADD: the sum of up to three terms, each maybe negated, the first maybe shifted left.
    IntAdd { d: Reg, terms: [Option<(Src, bool)>; 3], shift: Option<u8> },
    Xmad { d: Reg, a: Reg, b: Reg, c: Reg, a_signed: bool, b_signed: bool, a_hi: bool, b_hi: bool, psl: bool, mrg: bool, addend: XmadAddend },
    Lop { d: Reg, a: Reg, b: Src, op: LogicOp, inv_a: bool, inv_b: bool, pout: Option<(u8, ZeroTest)> },
    /// `wrap`: the shift count is taken mod 32 (otherwise 32 and over shift everything out).
    Shift { d: Reg, a: Reg, b: Src, left: bool, signed: bool, wrap: bool },
    /// `a` where `c` holds, `b` where not.
    Sel { d: Reg, a: Reg, b: Src, c: Pred },
    Mov { d: Reg, b: Src },
    I2f { d: Reg, b: Src, to: FloatFmt, from: IntFmt, byte: u8, m: Mods },
    F2i { d: Reg, b: Src, to: IntFmt, from: FloatFmt, round: Round, m: Mods },
    /// `int_round`: also round to an integral value; `high`: the upper half of a packed f16 source.
    F2f { d: Reg, b: Src, to: FloatFmt, from: FloatFmt, int_round: Option<Round>, high: bool, m: Mods, sat: bool },
    /// `cc`: also sets the condition code's zero and sign flags.
    I2i { d: Reg, b: Src, to: IntFmt, from: IntFmt, byte: u8, m: Mods, sat: bool, cc: bool },
    /// Predicate i = bit i of `a`, for the bits of `mask`.
    R2p { a: Reg, mask: u32 },
    /// Load `count` attribute words from byte `offset`.
    Ald { d: Reg, offset: u32, index: Reg, vertex: Reg, count: u8, output: bool, patch: bool },
    Ast { s: Reg, offset: u32, index: Reg, vertex: Reg, count: u8 },
    /// `count` words from `c[slot][index + offset]`.
    Ldc { d: Reg, slot: u8, index: Reg, offset: i32, count: u8 },
    Ipa { d: Reg, offset: u32, index: Reg, interp: Interp, mul: Reg, sat: bool },
    /// The first two components land in `d0` and the register after it, the rest in `d1` and the one after.
    Texs { d0: Reg, d1: Reg, a: Reg, b: Reg, handle: u16, shape: TexShape, components: &'static [u8] },
    /// textureQueryLod; `components` is the write mask.
    Tmml { d: Reg, a: Reg, handle: u16, components: u8 },
    Kil,
    Exit,
    Bra { target: u32 },
    Ssy { target: u32 },
    Sync,
    /// NOP and DEPBAR: nothing to translate.
    Nop,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instr {
    /// Byte address, counted from the first word after the program header (scheduling words included).
    pub addr: u32,
    /// The instruction runs only where this holds.
    pub guard: Pred,
    pub op: Op,
}
