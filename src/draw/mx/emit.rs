//! One instruction as WGSL statements, over the stage's state: registers `r<n>: u32`, predicates `p<n>: bool`, the
//! condition code's `ccz` / `ccs`, and the stage's inputs and outputs (`in_*` / `out_*`, see `wgsl`). Every
//! instruction reads and writes its registers as raw bits; floats are bitcast in and out.
//!
//! While writing, it notes what the stage turns out to use, which decides what the module declares.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::{anyhow, bail, Result};

use super::isa::*;
use super::wgsl::Stage;

/// What a stage reads and writes, found while its instructions are written.
#[derive(Default)]
pub struct Usage {
    /// Bit n: constant buffer c<n>.
    pub cbufs: u32,
    /// Bit n: generic input n.
    pub inputs: u32,
    /// Bit n: generic output n.
    pub outputs: u32,
    pub vertex_id: bool,
    pub units: BTreeSet<u32>,
    /// Highest register named.
    pub max_reg: u8,
    pub cc: bool,
    pub query_lod: bool,
}

pub struct Writer {
    pub stage: Stage,
    pub used: Usage,
}

/// A float constant, by its exact bits.
fn float_bits(v: f32) -> String {
    format!("bitcast<f32>({:#x}u)", v.to_bits())
}

fn modded(x: String, m: Mods) -> String {
    let x = if m.abs { format!("abs({x})") } else { x };
    if m.neg {
        format!("(-{x})")
    } else {
        x
    }
}

fn negated(x: String, neg: bool) -> String {
    modded(x, Mods { neg, abs: false })
}

fn saturated(x: String, sat: bool) -> String {
    if sat {
        format!("clamp({x}, 0.0, 1.0)")
    } else {
        x
    }
}

/// Unsigned negation, for the integer adds.
fn int_negated(x: String, neg: bool) -> String {
    if neg {
        format!("(0u - {x})")
    } else {
        x
    }
}

/// `r` combined with predicate `c`, folding the always-true predicate away.
fn combine(op: BoolOp, r: &str, c: &str) -> String {
    match (op, c) {
        (BoolOp::And, "true") => r.to_string(),
        (BoolOp::Or, "true") => "true".into(),
        (BoolOp::Xor, "true") => format!("!({r})"),
        (BoolOp::And, _) => format!("(({r}) && {c})"),
        (BoolOp::Or, _) => format!("(({r}) || {c})"),
        (BoolOp::Xor, _) => format!("(({r}) != {c})"),
    }
}

/// A float comparison. The unordered ones are written as the negated opposite, so NaN makes them hold.
fn compare_floats(cmp: FloatCmp, a: &str, b: &str) -> String {
    use FloatCmp::*;
    let ne = format!("{a} < {b} || {a} > {b}");
    let ordered = format!("{a} == {a} && {b} == {b}");
    match cmp {
        F => "false".into(),
        T => "true".into(),
        Lt => format!("({a} < {b})"),
        Eq => format!("({a} == {b})"),
        Le => format!("({a} <= {b})"),
        Gt => format!("({a} > {b})"),
        Ne => format!("({ne})"),
        Ge => format!("({a} >= {b})"),
        Num => format!("({ordered})"),
        Nan => format!("!({ordered})"),
        Ltu => format!("!({a} >= {b})"),
        Equ => format!("!({ne})"),
        Leu => format!("!({a} > {b})"),
        Gtu => format!("!({a} <= {b})"),
        Neu => format!("!({a} == {b})"),
        Geu => format!("!({a} < {b})"),
    }
}

fn compare_ints(cmp: IntCmp, signed: bool, a: &str, b: &str) -> String {
    let op = match cmp {
        IntCmp::F => return "false".into(),
        IntCmp::T => return "true".into(),
        IntCmp::Lt => "<",
        IntCmp::Eq => "==",
        IntCmp::Le => "<=",
        IntCmp::Gt => ">",
        IntCmp::Ne => "!=",
        IntCmp::Ge => ">=",
    };
    if signed {
        format!("(bitcast<i32>({a}) {op} bitcast<i32>({b}))")
    } else {
        format!("({a} {op} {b})")
    }
}

/// The value of format `fmt` held in the bits `x` (at byte `byte` for 8-bit formats, half `byte / 2` for 16-bit
/// ones), as an i32.
fn int_of(x: &str, fmt: IntFmt, byte: u8) -> String {
    let (byte8, half16) = (byte as u32 * 8, byte as u32 / 2 * 16);
    match fmt {
        IntFmt::U32 | IntFmt::S32 => format!("bitcast<i32>({x})"),
        IntFmt::U8 => format!("i32(extractBits({x}, {byte8}u, 8u))"),
        IntFmt::U16 => format!("i32(extractBits({x}, {half16}u, 16u))"),
        IntFmt::S8 => format!("extractBits(bitcast<i32>({x}), {byte8}u, 8u)"),
        IntFmt::S16 => format!("extractBits(bitcast<i32>({x}), {half16}u, 16u)"),
    }
}

fn rounded(x: String, round: Round) -> String {
    let f = match round {
        Round::Nearest => "round",
        Round::Down => "floor",
        Round::Up => "ceil",
        Round::Zero => "trunc",
    };
    format!("{f}({x})")
}

/// A float as the bits of a packed f16 pair (the low half).
fn as_f16(x: &str) -> String {
    format!("pack2x16float(vec2<f32>({x}, 0.0))")
}

/// Several statements as one block.
fn block(s: &str) -> String {
    format!("{{ {s} }}")
}

/// Which generic attribute and component a byte offset in attribute space is, if it is one.
fn generic(offset: u32) -> Option<(u32, u32)> {
    (0x80..0x280).contains(&offset).then(|| ((offset - 0x80) / 16, offset % 16 / 4))
}

const TRUE_BITS: &str = "0xffffffffu";
const ONE_BITS: &str = "0x3f800000u";

impl Writer {
    pub fn new(stage: Stage) -> Self {
        Writer { stage, used: Usage::default() }
    }

    fn r(&mut self, r: Reg) -> String {
        if r.is_zero() {
            return "0u".into();
        }
        self.used.max_reg = self.used.max_reg.max(r.0);
        format!("r{}", r.0)
    }

    fn f(&mut self, r: Reg) -> String {
        if r.is_zero() {
            return "0.0".into();
        }
        format!("bitcast<f32>({})", self.r(r))
    }

    fn src(&mut self, s: Src) -> String {
        match s {
            Src::Reg(r) => self.r(r),
            Src::Cbuf { slot, offset } => {
                self.used.cbufs |= 1 << slot;
                format!("cb{slot}[{}]", offset / 4)
            }
            Src::Imm(bits) => format!("{bits:#x}u"),
        }
    }

    fn fsrc(&mut self, s: Src) -> String {
        if let Src::Imm(bits) = s {
            return float_bits(f32::from_bits(bits));
        }
        format!("bitcast<f32>({})", self.src(s))
    }

    pub fn pred(&self, p: Pred) -> String {
        let name = if p.index == Pred::PT { "true".to_string() } else { format!("p{}", p.index) };
        match (p.index == Pred::PT, p.not) {
            (true, true) => "false".into(),
            (_, true) => format!("!{name}"),
            (_, false) => name,
        }
    }

    /// `d = bits;`, or nothing when `d` is RZ.
    fn assign(&mut self, d: Reg, bits: String) -> String {
        if d.is_zero() {
            return String::new();
        }
        format!("{} = {bits};", self.r(d))
    }

    fn assign_f(&mut self, d: Reg, value: String) -> String {
        self.assign(d, format!("bitcast<u32>({value})"))
    }

    fn assign_p(&self, p: u8, value: &str) -> String {
        if p == Pred::PT {
            return String::new();
        }
        format!("p{p} = {value};")
    }

    fn sampler(&mut self, handle: u16) -> Result<u32> {
        // handles 8, 10, 12 ... are the bound texture units 0, 1, 2 ...
        if handle < 8 || handle % 2 != 0 {
            bail!("texture handle {handle} is not a bound unit");
        }
        let unit = (handle as u32 - 8) / 2;
        self.used.units.insert(unit);
        Ok(unit)
    }

    fn condition(&mut self, cond: Cond) -> Result<String> {
        Ok(match cond {
            Cond::Float { a, am, b, bm, cmp } => {
                let a = modded(self.f(a), am);
                let b = modded(self.fsrc(b), bm);
                compare_floats(cmp, &a, &b)
            }
            Cond::Int { a, b, cmp, signed } => {
                let (a, b) = (self.r(a), self.src(b));
                compare_ints(cmp, signed, &a, &b)
            }
            Cond::Preds { a, b, op } => combine(op, &self.pred(a), &self.pred(b)),
            Cond::Cc(test) => {
                self.used.cc = true;
                match test {
                    0 => "false",
                    15 => "true",
                    2 | 10 => "ccz",
                    5 | 13 => "!ccz",
                    t => bail!("condition code test {t}"),
                }
                .into()
            }
        })
    }

    /// The statements of one instruction. `sync_target` is where a SYNC goes.
    pub fn statement(&mut self, op: Op, sync_target: Option<u32>) -> Result<String> {
        Ok(match op {
            Op::Fadd { d, a, am, b, bm, sat } => {
                let a = modded(self.f(a), am);
                let b = modded(self.fsrc(b), bm);
                self.assign_f(d, saturated(format!("({a} + {b})"), sat))
            }
            Op::Fmul { d, a, b, neg_b, sat, scale, fmz } => {
                let a = self.f(a);
                let b = negated(self.fsrc(b), neg_b);
                let mut x = if fmz { format!("fmz({a}, {b})") } else { format!("({a} * {b})") };
                if let Some(k) = scale {
                    x = format!("({x} * {})", float_bits(k));
                }
                self.assign_f(d, saturated(x, sat))
            }
            Op::Ffma { d, a, b, c, neg_b, neg_c, sat, fmz } => {
                let a = self.f(a);
                let b = negated(self.fsrc(b), neg_b);
                let c = negated(self.fsrc(c), neg_c);
                let x = if fmz { format!("(fmz({a}, {b}) + {c})") } else { format!("fma({a}, {b}, {c})") };
                self.assign_f(d, saturated(x, sat))
            }
            Op::Fmnmx { d, a, am, b, bm, min } => {
                let a = modded(self.f(a), am);
                let b = modded(self.fsrc(b), bm);
                let x = match self.pred(min).as_str() {
                    "true" => format!("min({a}, {b})"),
                    "false" => format!("max({a}, {b})"),
                    p => format!("select(max({a}, {b}), min({a}, {b}), {p})"),
                };
                self.assign_f(d, x)
            }
            Op::Mufu { d, a, m, func, sat } => {
                let a = modded(self.f(a), m);
                let x = match func {
                    Func::Cos => format!("cos({a})"),
                    Func::Sin => format!("sin({a})"),
                    Func::Ex2 => format!("exp2({a})"),
                    Func::Lg2 => format!("log2({a})"),
                    Func::Rcp => format!("(1.0 / {a})"),
                    Func::Rsq => format!("inverseSqrt({a})"),
                    Func::Sqrt => format!("sqrt({a})"),
                };
                self.assign_f(d, saturated(x, sat))
            }
            Op::Rro { d, b, m } => {
                let b = modded(self.fsrc(b), m);
                self.assign_f(d, b)
            }
            Op::Set { cond, combine: op, with, dest } => {
                let r = self.condition(cond)?;
                let with = self.pred(with);
                match dest {
                    CondDest::Reg { d, float_one } => {
                        let one = if float_one { ONE_BITS } else { TRUE_BITS };
                        self.assign(d, format!("select(0u, {one}, {})", combine(op, &r, &with)))
                    }
                    CondDest::Preds { p, q } => {
                        let p = self.assign_p(p, &combine(op, "r", &with));
                        let q = self.assign_p(q, &combine(op, "!r", &with));
                        block(&format!("let r = {r}; {p} {q}"))
                    }
                }
            }
            Op::IntAdd { d, terms, shift } => {
                let mut parts = Vec::new();
                for (k, (s, neg)) in terms.into_iter().flatten().enumerate() {
                    let term = int_negated(self.src(s), neg);
                    parts.push(match shift {
                        Some(n) if k == 0 => format!("({term} << {n}u)"),
                        _ => term,
                    });
                }
                self.assign(d, format!("({})", parts.join(" + ")))
            }
            Op::Xmad { d, a, b, c, a_signed, b_signed, a_hi, b_hi, psl, mrg, addend } => {
                let (a, b, c) = (self.r(a), self.r(b), self.r(c));
                let half = |x: &str, hi: bool, signed: bool| {
                    let at = if hi { 16 } else { 0 };
                    if signed {
                        format!("extractBits(bitcast<i32>({x}), {at}u, 16u)")
                    } else {
                        format!("bitcast<i32>(extractBits({x}, {at}u, 16u))")
                    }
                };
                let mut product = format!("bitcast<u32>({} * {})", half(&a, a_hi, a_signed), half(&b, b_hi, b_signed));
                if psl {
                    product = format!("({product} << 16u)");
                }
                let addend = match addend {
                    XmadAddend::C => c,
                    XmadAddend::CLo => format!("({c} & 0xffffu)"),
                    XmadAddend::CHi => format!("({c} >> 16u)"),
                    XmadAddend::CBcc => format!("({c} + ({b} << 16u))"),
                    XmadAddend::CSfu => bail!("XMAD .csfu"),
                };
                let sum = format!("({product} + {addend})");
                let x = if mrg { format!("(({sum} & 0xffffu) | ({b} << 16u))") } else { sum };
                self.assign(d, x)
            }
            Op::Lop { d, a, b, op, inv_a, inv_b, pout } => {
                let invert = |x: String, on: bool| if on { format!("(~{x})") } else { x };
                let a = invert(self.r(a), inv_a);
                let b = invert(self.src(b), inv_b);
                let x = match op {
                    LogicOp::And => format!("({a} & {b})"),
                    LogicOp::Or => format!("({a} | {b})"),
                    LogicOp::Xor => format!("({a} ^ {b})"),
                    LogicOp::PassB => b,
                };
                let mut s = format!("let t = {x}; {}", self.assign(d, "t".into()));
                if let Some((p, test)) = pout {
                    let v = match test {
                        ZeroTest::Always => "true",
                        ZeroTest::Zero => "t == 0u",
                        ZeroTest::NonZero => "t != 0u",
                    };
                    s += &self.assign_p(p, v);
                }
                block(&s)
            }
            Op::Shift { d, a, b, left, signed, wrap } => {
                let (a, b) = (self.r(a), self.src(b));
                let x = match (left, signed, wrap) {
                    (true, _, true) => format!("({a} << ({b} & 31u))"),
                    (true, _, false) => format!("select(0u, {a} << ({b} & 31u), {b} < 32u)"),
                    (false, false, true) => format!("({a} >> ({b} & 31u))"),
                    (false, false, false) => format!("select(0u, {a} >> ({b} & 31u), {b} < 32u)"),
                    (false, true, true) => format!("bitcast<u32>(bitcast<i32>({a}) >> ({b} & 31u))"),
                    (false, true, false) => format!("bitcast<u32>(bitcast<i32>({a}) >> min({b}, 31u))"),
                };
                self.assign(d, x)
            }
            Op::Sel { d, a, b, c } => {
                let (a, b) = (self.r(a), self.src(b));
                let c = self.pred(c);
                self.assign(d, format!("select({b}, {a}, {c})"))
            }
            Op::Mov { d, b } => {
                let b = self.src(b);
                self.assign(d, b)
            }
            Op::I2f { d, b, to, from, byte, m } => {
                let b = self.src(b);
                let v = match from {
                    IntFmt::U32 => format!("f32({b})"),
                    IntFmt::S32 => format!("f32(bitcast<i32>({b}))"),
                    f => format!("f32({})", int_of(&b, f, byte)),
                };
                let v = modded(v, m);
                match to {
                    FloatFmt::F32 => self.assign_f(d, v),
                    FloatFmt::F16 => self.assign(d, as_f16(&v)),
                }
            }
            Op::F2i { d, b, to, from, round, m } => {
                let b = self.src(b);
                let v = match from {
                    FloatFmt::F32 => format!("bitcast<f32>({b})"),
                    FloatFmt::F16 => format!("unpack2x16float({b}).x"),
                };
                let v = rounded(modded(v, m), round);
                // the hardware clamps to the format's range, and NaN converts to 0
                let (lo, hi, signed) = match to {
                    IntFmt::S32 => ("-2147483648.0", "2147483520.0", true),
                    IntFmt::U32 => ("0.0", "4294967040.0", false),
                    IntFmt::U16 => ("0.0", "65535.0", false),
                    IntFmt::S16 => ("-32768.0", "32767.0", true),
                    IntFmt::U8 => ("0.0", "255.0", false),
                    IntFmt::S8 => ("-128.0", "127.0", true),
                };
                let clamped = format!("clamp({v}, {lo}, {hi})");
                let x = if signed { format!("bitcast<u32>(i32({clamped}))") } else { format!("u32({clamped})") };
                self.assign(d, format!("select(0u, {x}, {v} == {v})"))
            }
            Op::F2f { d, b, to, from, int_round, high, m, sat } => {
                let b = self.src(b);
                let v = match from {
                    FloatFmt::F32 => format!("bitcast<f32>({b})"),
                    FloatFmt::F16 => format!("unpack2x16float({b}).{}", if high { "y" } else { "x" }),
                };
                let mut v = modded(v, m);
                if let Some(round) = int_round {
                    v = rounded(v, round);
                }
                let v = saturated(v, sat);
                match to {
                    FloatFmt::F32 => self.assign_f(d, v),
                    FloatFmt::F16 => self.assign(d, as_f16(&v)),
                }
            }
            Op::I2i { d, b, to, from, byte, m, sat, cc } => {
                let b = self.src(b);
                let v = modded(int_of(&b, from, byte), m);
                let x = match (to, sat) {
                    (IntFmt::U32 | IntFmt::S32, false) => format!("bitcast<u32>({v})"),
                    (IntFmt::S32, true) if from.unsigned() => format!("min(bitcast<u32>({v}), 0x7fffffffu)"),
                    (IntFmt::S32, true) => format!("bitcast<u32>({v})"),
                    (IntFmt::U32, true) if from.unsigned() => format!("bitcast<u32>({v})"),
                    (IntFmt::U32, true) => format!("bitcast<u32>(max({v}, 0))"),
                    (IntFmt::U16, false) => format!("(bitcast<u32>({v}) & 0xffffu)"),
                    (IntFmt::U8, false) => format!("(bitcast<u32>({v}) & 0xffu)"),
                    (IntFmt::S16, false) => format!("bitcast<u32>(extractBits({v}, 0u, 16u))"),
                    (IntFmt::S8, false) => format!("bitcast<u32>(extractBits({v}, 0u, 8u))"),
                    (f, true) => bail!("I2I saturating to {f:?}"),
                };
                let mut s = format!("let t = {x}; {}", self.assign(d, "t".into()));
                if cc {
                    self.used.cc = true;
                    s += " ccz = t == 0u; ccs = bitcast<i32>(t) < 0;";
                }
                block(&s)
            }
            Op::R2p { a, mask } => {
                let a = self.r(a);
                let mut s = String::new();
                for i in (0..7).filter(|i| mask >> i & 1 != 0) {
                    let _ = write!(s, "p{i} = ({a} & {:#x}u) != 0u; ", 1u32 << i);
                }
                format!("{{ {s}}}")
            }
            Op::Ald { d, offset, index, vertex, count, output, patch } => {
                if self.stage != Stage::Vertex || !index.is_zero() || !vertex.is_zero() || output || patch {
                    bail!("only plain vertex attribute loads are supported");
                }
                let mut s = String::new();
                for k in 0..count {
                    let at = offset + 4 * k as u32;
                    let v = match generic(at) {
                        Some((n, c)) => {
                            self.used.inputs |= 1 << n;
                            format!("bitcast<u32>(in_a{n}[{c}])")
                        }
                        None if at == 0x2fc => {
                            self.used.vertex_id = true;
                            "in_vertex_id".into()
                        }
                        None => bail!("attribute load from {at:#x}"),
                    };
                    s += &self.assign(d.nth(k), v);
                }
                s
            }
            Op::Ast { s: from, offset, index, vertex, count } => {
                if self.stage != Stage::Vertex || !index.is_zero() || !vertex.is_zero() {
                    bail!("only plain vertex output stores are supported");
                }
                let mut s = String::new();
                for k in 0..count {
                    let at = offset + 4 * k as u32;
                    let v = self.f(from.nth(k));
                    if (0x70..0x80).contains(&at) {
                        let _ = write!(s, "out_pos[{}] = {v}; ", (at - 0x70) / 4);
                    } else if let Some((n, c)) = generic(at) {
                        self.used.outputs |= 1 << n;
                        let _ = write!(s, "out_v{n}[{c}] = {v}; ");
                    } else if at != 0x6c {
                        // (0x6c is the point size; WGSL points are always one pixel)
                        bail!("attribute store to {at:#x}");
                    }
                }
                s
            }
            Op::Ldc { d, slot, index, offset, count } => {
                self.used.cbufs |= 1 << slot;
                let i = self.r(index);
                let mut s = format!("let at = u32(bitcast<i32>({i}) + {offset}) >> 2u; ");
                for k in 0..count {
                    s += &self.assign(d.nth(k), format!("cb{slot}[at + {k}u]"));
                }
                block(&s)
            }
            Op::Ipa { d, offset, index, interp, mul, sat } => {
                if self.stage != Stage::Fragment || !index.is_zero() {
                    bail!("only plain fragment interpolation is supported");
                }
                let v = match generic(offset) {
                    Some((n, c)) => {
                        self.used.inputs |= 1 << n;
                        format!("in_v{n}[{c}]")
                    }
                    // 1/w, for perspective correction; wgpu's varyings are already corrected, so it is 1
                    None if offset == 0x7c => "1.0".into(),
                    None if (0x70..0x7c).contains(&offset) => format!("in_pos[{}]", (offset - 0x70) / 4),
                    None => bail!("interpolation of {offset:#x}"),
                };
                let v = match interp {
                    Interp::Multiply => format!("({v} * {})", self.f(mul)),
                    Interp::Pass | Interp::Constant => v,
                    Interp::Sc => bail!("IPA .sc"),
                };
                self.assign_f(d, saturated(v, sat))
            }
            Op::Texs { d0, d1, a, b, handle, shape, components } => {
                let unit = self.sampler(handle)?;
                let (u, v, lod) = match shape {
                    TexShape::Tex2D => (self.f(a), self.f(b), None),
                    TexShape::Tex2DLz => (self.f(a), self.f(b), Some("0.0".to_string())),
                    TexShape::Tex2DLl => (self.f(a), self.f(a.nth(1)), Some(self.f(b))),
                    other => bail!("TEXS {other:?}"),
                };
                // a vertex stage has no derivatives, so it samples the top level
                let lod = lod.or_else(|| (self.stage == Stage::Vertex).then(|| "0.0".to_string()));
                let uv = format!("vec2<f32>({u}, {v})");
                let sample = match lod {
                    None => format!("textureSample(tex{unit}, smp{unit}, {uv})"),
                    Some(l) => format!("textureSampleLevel(tex{unit}, smp{unit}, {uv}, {l})"),
                };
                let mut s = format!("let t = {sample}; ");
                let dests = [d0, d0.nth(1), d1, d1.nth(1)];
                for (k, &c) in components.iter().enumerate() {
                    if k >= 2 && d1.is_zero() {
                        bail!("TEXS component {k} with no second destination");
                    }
                    s += &self.assign_f(dests[k], format!("t[{c}]"));
                }
                block(&s)
            }
            Op::Tmml { d, a, handle, components } => {
                if self.stage != Stage::Fragment {
                    bail!("level of detail query outside a fragment stage");
                }
                let unit = self.sampler(handle)?;
                self.used.query_lod = true;
                let (u, v) = (self.f(a), self.f(a.nth(1)));
                let mut s = format!("let q = query_lod(tex{unit}, vec2<f32>({u}, {v})); ");
                let mut next = d;
                for c in (0..2).filter(|c| components >> c & 1 != 0) {
                    s += &self.assign(next, format!("bitcast<u32>(i32(q[{c}] * 256.0))"));
                    next = next.nth(1);
                }
                if components >> 2 != 0 {
                    bail!("level of detail components {components:#b}");
                }
                block(&s)
            }
            Op::Kil => {
                if self.stage != Stage::Fragment {
                    bail!("discard outside a fragment stage");
                }
                "discard;".into()
            }
            Op::Exit => match self.stage {
                // render target 0 is registers 0..3 when the program ends
                Stage::Fragment => {
                    let rgba: Vec<String> = (0..4).map(|i| self.f(Reg(i))).collect();
                    format!("out_color = vec4<f32>({}); return;", rgba.join(", "))
                }
                Stage::Vertex => "return;".into(),
            },
            Op::Bra { target } => format!("pc = {target}u; continue;"),
            Op::Sync => {
                let target = sync_target.ok_or_else(|| anyhow!("SYNC no path reaches"))?;
                format!("pc = {target}u; continue;")
            }
            Op::Ssy { .. } | Op::Nop => String::new(),
        })
    }
}
