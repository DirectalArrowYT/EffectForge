//! Decoded effect shader stages, written out as WGSL for wgpu.
//!
//! The translation is literal: every register is a `u32` variable, every predicate a `bool`, and every instruction
//! one statement that reads and writes them. Control flow keeps the machine's shape: when the program branches at
//! all, its code is cut into blocks at every branch target and runs as `loop { switch pc { ... } }`. SSY / SYNC (the
//! hardware's reconvergence stack) is resolved statically: each SYNC jumps to the SSY target in force where it is.
//!
//! Interface (the binding layout eff_gpu builds):
//! * set 0: constant buffer `c<n>` of the vertex stage at binding n, the fragment stage's at 20 + n, each a read-only
//!   storage buffer of `u32`; the fragment stage's alpha test block (func, ref, point size, 0) at 39.
//! * set 1: texture unit u of the fragment stage at binding u, of the vertex stage at 32 + u; its sampler 64 above.
//! * vertex inputs: generic attribute n at location n as a `vec4<f32>`; varyings: generic output n at location n.
//! * fragment output: render target 0 at location 0 (registers 0..3 at EXIT, as the program header maps it).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{anyhow, bail, Context, Result};

use super::decode::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
}

pub const FRAGMENT_CBUF_BASE: u32 = 20;
pub const VERTEX_TEXTURE_BASE: u32 = 32;
pub const SAMPLER_SPLIT: u32 = 64;
pub const ALPHA_TEST_BINDING: u32 = 39;

/// One translated stage and what a draw has to give it.
#[derive(Clone, Debug)]
pub struct Shader {
    pub wgsl: String,
    /// Bit n: reads constant buffer c<n>.
    pub cbufs: u32,
    /// Bit n: reads generic input n (vertex attribute / varying).
    pub inputs: u32,
    /// Bit n: writes varying n (vertex stage).
    pub outputs: u32,
    /// Texture units sampled, ascending.
    pub units: Vec<u32>,
}

/// The program of an NVN code section (`code`, magic at 0, program header at 0x30) whose control section is `ctl`.
pub fn program<'a>(code: &'a [u8], ctl: &[u8]) -> Result<&'a [u8]> {
    let len = ctl.get(0x6f8..0x6fc).ok_or_else(|| anyhow!("short control section"))?;
    let len = u32::from_le_bytes(len.try_into().unwrap()) as usize;
    code.get(0x30..0x30 + len).filter(|p| p.len() >= 0x50).ok_or_else(|| anyhow!("short code section"))
}

/// Translate one stage. `program` = the program header and code (see [`program`]). A vertex stage also writes the
/// varyings in `extra_outputs` (as (0, 0, 0, 1) where it does not write them itself), so it can feed a fragment
/// stage that reads more than it writes.
pub fn translate(program: &[u8], stage: Stage, extra_outputs: u32) -> Result<Shader> {
    let header: Vec<u32> = program[..0x50].chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let kind = header[0] >> 10 & 15;
    match (stage, kind) {
        (Stage::Vertex, 1) | (Stage::Fragment, 5) => {}
        _ => bail!("program header says shader type {kind}"),
    }
    if stage == Stage::Fragment && (header[18] != 0xf || header[19] & 3 != 0) {
        bail!("fragment outputs {:#x} / {:#x} (only render target 0, RGBA)", header[18], header[19]);
    }
    let code = decode_program(&program[0x50..])?;
    Emitter::new(stage, &code, extra_outputs)?.run()
}

/// The SSY target in force at each instruction (by index), following every path from the entry.
fn sync_targets(code: &[Instr], index: &BTreeMap<u32, usize>) -> Result<BTreeMap<usize, u32>> {
    let mut stack_at: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
    let mut targets = BTreeMap::new();
    let mut work = vec![(0usize, Vec::<u32>::new())];
    let at = |addr: u32| index.get(&addr).copied().ok_or_else(|| anyhow!("jump to {addr:#x}, outside the program"));
    while let Some((i, stack)) = work.pop() {
        if i >= code.len() {
            continue;
        }
        match stack_at.get(&i) {
            Some(s) if *s == stack => continue,
            Some(s) => bail!("{:#x} is reached with different SSY stacks ({s:x?} / {stack:x?})", code[i].addr),
            None => {}
        }
        stack_at.insert(i, stack.clone());
        let ins = &code[i];
        let always = ins.guard.is_true();
        if ins.guard.is_false() {
            work.push((i + 1, stack));
            continue;
        }
        match ins.op {
            Op::Exit => {
                if !always {
                    work.push((i + 1, stack));
                }
            }
            Op::Bra { target } => {
                work.push((at(target)?, stack.clone()));
                if !always {
                    work.push((i + 1, stack));
                }
            }
            Op::Ssy { target } => {
                let mut s = stack;
                s.push(target);
                work.push((i + 1, s));
            }
            Op::Sync => {
                let mut s = stack.clone();
                let target = s.pop().ok_or_else(|| anyhow!("SYNC at {:#x} with no SSY", ins.addr))?;
                targets.insert(i, target);
                work.push((at(target)?, s));
                if !always {
                    work.push((i + 1, stack));
                }
            }
            _ => work.push((i + 1, stack)),
        }
    }
    Ok(targets)
}

struct Emitter<'a> {
    stage: Stage,
    code: &'a [Instr],
    extra_outputs: u32,
    cbufs: u32,
    inputs: u32,
    outputs: u32,
    position: bool,
    vertex_id: bool,
    units: BTreeSet<u32>,
    max_reg: u8,
    cc: bool,
    tmml: bool,
    body: String,
}

fn fbits(v: f32) -> String {
    format!("bitcast<f32>({:#x}u)", v.to_bits())
}

/// Combine a comparison result with predicate `c`.
fn bool_op(op: BoolOp, r: &str, c: &str) -> String {
    if c == "true" {
        return match op {
            BoolOp::And => r.to_string(),
            BoolOp::Or => "true".into(),
            BoolOp::Xor => format!("!({r})"),
        };
    }
    match op {
        BoolOp::And => format!("(({r}) && {c})"),
        BoolOp::Or => format!("(({r}) || {c})"),
        BoolOp::Xor => format!("(({r}) != {c})"),
    }
}

fn float_cmp(cmp: FloatCmp, a: &str, b: &str) -> String {
    use FloatCmp::*;
    match cmp {
        F => "false".into(),
        Lt => format!("({a} < {b})"),
        Eq => format!("({a} == {b})"),
        Le => format!("({a} <= {b})"),
        Gt => format!("({a} > {b})"),
        Ne => format!("({a} < {b} || {a} > {b})"),
        Ge => format!("({a} >= {b})"),
        Num => format!("({a} == {a} && {b} == {b})"),
        Nan => format!("!({a} == {a} && {b} == {b})"),
        Ltu => format!("!({a} >= {b})"),
        Equ => format!("!({a} < {b} || {a} > {b})"),
        Leu => format!("!({a} > {b})"),
        Gtu => format!("!({a} <= {b})"),
        Neu => format!("!({a} == {b})"),
        Geu => format!("!({a} < {b})"),
        T => "true".into(),
    }
}

fn int_cmp(cmp: IntCmp, signed: bool, a: &str, b: &str) -> String {
    let (a, b) = if signed { (format!("bitcast<i32>({a})"), format!("bitcast<i32>({b})")) } else { (a.into(), b.into()) };
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
    format!("({a} {op} {b})")
}

fn fmod(x: String, neg: bool, abs: bool) -> String {
    let x = if abs { format!("abs({x})") } else { x };
    if neg {
        format!("(-{x})")
    } else {
        x
    }
}

fn sat(x: String, on: bool) -> String {
    if on {
        format!("clamp({x}, 0.0, 1.0)")
    } else {
        x
    }
}

/// An integer value of format `fmt` taken from the 32 bits `x` (byte `byte` for the 8-bit formats, half `byte / 2`
/// for the 16-bit ones), as an i32 expression.
fn int_value(x: &str, fmt: IntFmt, byte: u8) -> String {
    let byte = byte as u32;
    match fmt {
        IntFmt::U32 | IntFmt::S32 => format!("bitcast<i32>({x})"),
        IntFmt::U8 => format!("i32(extractBits({x}, {}u, 8u))", byte * 8),
        IntFmt::U16 => format!("i32(extractBits({x}, {}u, 16u))", byte / 2 * 16),
        IntFmt::S8 => format!("extractBits(bitcast<i32>({x}), {}u, 8u)", byte * 8),
        IntFmt::S16 => format!("extractBits(bitcast<i32>({x}), {}u, 16u)", byte / 2 * 16),
    }
}

impl<'a> Emitter<'a> {
    fn new(stage: Stage, code: &'a [Instr], extra_outputs: u32) -> Result<Self> {
        Ok(Self {
            stage,
            code,
            extra_outputs,
            cbufs: 0,
            inputs: 0,
            outputs: 0,
            position: false,
            vertex_id: false,
            units: BTreeSet::new(),
            max_reg: 0,
            cc: false,
            tmml: false,
            body: String::new(),
        })
    }

    fn reg(&mut self, r: Reg) -> String {
        if r.is_zero() {
            "0u".into()
        } else {
            self.max_reg = self.max_reg.max(r.0);
            format!("r{}", r.0)
        }
    }

    fn freg(&mut self, r: Reg) -> String {
        if r.is_zero() {
            "0.0".into()
        } else {
            format!("bitcast<f32>({})", self.reg(r))
        }
    }

    fn src(&mut self, s: Src) -> String {
        match s {
            Src::Reg(r) => self.reg(r),
            Src::Cbuf { slot, offset } => {
                self.cbufs |= 1 << slot;
                format!("cb{slot}[{}]", offset / 4)
            }
            Src::Imm(v) => format!("{v:#x}u"),
        }
    }

    fn fsrc(&mut self, s: Src) -> String {
        match s {
            Src::Imm(v) => fbits(f32::from_bits(v)),
            s => format!("bitcast<f32>({})", self.src(s)),
        }
    }

    fn pred(&self, p: Pred) -> String {
        match (p.index, p.not) {
            (7, false) => "true".into(),
            (7, true) => "false".into(),
            (i, false) => format!("p{i}"),
            (i, true) => format!("!p{i}"),
        }
    }

    /// `d = expr` (u32) as a statement; nothing for RZ.
    fn set(&mut self, d: Reg, expr: String) -> String {
        if d.is_zero() {
            String::new()
        } else {
            format!("{} = {expr};", self.reg(d))
        }
    }

    fn setf(&mut self, d: Reg, expr: String) -> String {
        self.set(d, format!("bitcast<u32>({expr})"))
    }

    fn setp(&self, p: u8, expr: &str) -> String {
        if p == 7 {
            String::new()
        } else {
            format!("p{p} = {expr};")
        }
    }

    fn generic(offset: u32) -> Option<(u32, u32)> {
        (0x80..0x280).contains(&offset).then(|| ((offset - 0x80) / 16, offset % 16 / 4))
    }

    fn texture(&mut self, field: u16) -> Result<(String, String)> {
        if field < 8 || field % 2 != 0 {
            bail!("texture handle {field} is not a bound texture unit");
        }
        let unit = (field as u32 - 8) / 2;
        self.units.insert(unit);
        Ok((format!("tex{unit}"), format!("smp{unit}")))
    }

    /// The statement(s) of one instruction; `next` is the address after it (for SYNC: its resolved target).
    fn instr(&mut self, ins: &Instr, sync_target: Option<u32>) -> Result<String> {
        let s = match ins.op {
            Op::Fadd { d, a, b, neg_a, abs_a, neg_b, abs_b, sat: s, .. } => {
                let a = fmod(self.freg(a), neg_a, abs_a);
                let b = fmod(self.fsrc(b), neg_b, abs_b);
                self.setf(d, sat(format!("({a} + {b})"), s))
            }
            Op::Fmul { d, a, b, neg_b, sat: s, scale, fmz, .. } => {
                let a = self.freg(a);
                let b = fmod(self.fsrc(b), neg_b, false);
                let mut x = if fmz { format!("fmz({a}, {b})") } else { format!("({a} * {b})") };
                let k = match scale {
                    FmulScale::None => None,
                    FmulScale::D2 => Some(0.5),
                    FmulScale::D4 => Some(0.25),
                    FmulScale::D8 => Some(0.125),
                    FmulScale::M8 => Some(8.0),
                    FmulScale::M4 => Some(4.0),
                    FmulScale::M2 => Some(2.0),
                };
                if let Some(k) = k {
                    x = format!("({x} * {})", fbits(k));
                }
                self.setf(d, sat(x, s))
            }
            Op::Ffma { d, a, b, c, neg_b, neg_c, sat: s, fmz, .. } => {
                let a = self.freg(a);
                let b = fmod(self.fsrc(b), neg_b, false);
                let c = fmod(self.fsrc(c), neg_c, false);
                let x = if fmz { format!("(fmz({a}, {b}) + {c})") } else { format!("fma({a}, {b}, {c})") };
                self.setf(d, sat(x, s))
            }
            Op::Fmnmx { d, a, b, neg_a, abs_a, neg_b, abs_b, min } => {
                let a = fmod(self.freg(a), neg_a, abs_a);
                let b = fmod(self.fsrc(b), neg_b, abs_b);
                let x = match self.pred(min).as_str() {
                    "true" => format!("min({a}, {b})"),
                    "false" => format!("max({a}, {b})"),
                    p => format!("select(max({a}, {b}), min({a}, {b}), {p})"),
                };
                self.setf(d, x)
            }
            Op::Mufu { d, a, op, neg, abs, sat: s } => {
                let a = fmod(self.freg(a), neg, abs);
                let x = match op {
                    MufuOp::Cos => format!("cos({a})"),
                    MufuOp::Sin => format!("sin({a})"),
                    MufuOp::Ex2 => format!("exp2({a})"),
                    MufuOp::Lg2 => format!("log2({a})"),
                    MufuOp::Rcp => format!("(1.0 / {a})"),
                    MufuOp::Rsq => format!("inverseSqrt({a})"),
                    MufuOp::Sqrt => format!("sqrt({a})"),
                };
                self.setf(d, sat(x, s))
            }
            // the range reduction MUFU sin / cos / ex2 want on the hardware; the WGSL functions take the plain value
            Op::Rro { d, b, neg, abs } => {
                let b = fmod(self.fsrc(b), neg, abs);
                self.setf(d, b)
            }
            Op::Fset { d, a, b, cmp, bop, c, neg_a, abs_a, neg_b, abs_b, bf } => {
                let a = fmod(self.freg(a), neg_a, abs_a);
                let b = fmod(self.fsrc(b), neg_b, abs_b);
                let r = bool_op(bop, &float_cmp(cmp, &a, &b), &self.pred(c));
                let one = if bf { "0x3f800000u" } else { "0xffffffffu" };
                self.set(d, format!("select(0u, {one}, {r})"))
            }
            Op::Fsetp { p, q, a, b, cmp, bop, c, neg_a, abs_a, neg_b, abs_b } => {
                let a = fmod(self.freg(a), neg_a, abs_a);
                let b = fmod(self.fsrc(b), neg_b, abs_b);
                self.setp_pair(p, q, &float_cmp(cmp, &a, &b), bop, c)
            }
            Op::Csetp { p, q, test, bop, c } => {
                let r = match test {
                    0 => "false",
                    15 => "true",
                    2 | 10 => "ccz",
                    5 | 13 => "!ccz",
                    t => bail!("condition code test {t}"),
                };
                self.cc = true;
                self.setp_pair(p, q, r, bop, c)
            }
            Op::Psetp { p, q, a, b, c, op1, op2 } => {
                let r = bool_op(op1, &self.pred(a), &self.pred(b));
                self.setp_pair(p, q, &r, op2, c)
            }
            Op::Pset { d, a, b, c, op1, op2, bf } => {
                let r = bool_op(op2, &bool_op(op1, &self.pred(a), &self.pred(b)), &self.pred(c));
                let one = if bf { "0x3f800000u" } else { "0xffffffffu" };
                self.set(d, format!("select(0u, {one}, {r})"))
            }
            Op::Iset { d, a, b, cmp, signed, bop, c, bf } => {
                let (a, b) = (self.reg(a), self.src(b));
                let r = bool_op(bop, &int_cmp(cmp, signed, &a, &b), &self.pred(c));
                let one = if bf { "0x3f800000u" } else { "0xffffffffu" };
                self.set(d, format!("select(0u, {one}, {r})"))
            }
            Op::Isetp { p, q, a, b, cmp, signed, bop, c } => {
                let (a, b) = (self.reg(a), self.src(b));
                self.setp_pair(p, q, &int_cmp(cmp, signed, &a, &b), bop, c)
            }
            Op::Iadd { d, a, b, neg_a, neg_b } => {
                let a = self.reg(a);
                let b = self.src(b);
                let a = if neg_a { format!("(0u - {a})") } else { a };
                let b = if neg_b { format!("(0u - {b})") } else { b };
                self.set(d, format!("({a} + {b})"))
            }
            Op::Iadd3 { d, a, b, c, neg_a, neg_b, neg_c } => {
                let mut terms = Vec::new();
                for (r, n) in [(a, neg_a), (b, neg_b), (c, neg_c)] {
                    let r = self.reg(r);
                    terms.push(if n { format!("(0u - {r})") } else { r });
                }
                self.set(d, format!("({})", terms.join(" + ")))
            }
            Op::Iscadd { d, a, b, shift, neg_a, neg_b } => {
                let a = self.reg(a);
                let b = self.src(b);
                let a = if neg_a { format!("(0u - {a})") } else { a };
                let b = if neg_b { format!("(0u - {b})") } else { b };
                self.set(d, format!("(({a} << {shift}u) + {b})"))
            }
            Op::Xmad { d, a, b, c, a_signed, b_signed, a_hi, b_hi, psl, mrg, mode } => {
                let (ra, rb, rc) = (self.reg(a), self.src(b), self.src(c));
                let half = |x: &str, hi: bool, signed: bool| {
                    if signed {
                        format!("extractBits(bitcast<i32>({x}), {}u, 16u)", if hi { 16 } else { 0 })
                    } else {
                        format!("bitcast<i32>(extractBits({x}, {}u, 16u))", if hi { 16 } else { 0 })
                    }
                };
                let product = format!("bitcast<u32>({} * {})", half(&ra, a_hi, a_signed), half(&rb, b_hi, b_signed));
                let product = if psl { format!("({product} << 16u)") } else { product };
                let addend = match mode {
                    XmadMode::C => rc,
                    XmadMode::CLo => format!("({rc} & 0xffffu)"),
                    XmadMode::CHi => format!("({rc} >> 16u)"),
                    XmadMode::CBcc => format!("({rc} + ({rb} << 16u))"),
                    XmadMode::CSfu => bail!("xmad .csfu"),
                };
                let sum = format!("({product} + {addend})");
                let x = if mrg { format!("(({sum} & 0xffffu) | ({rb} << 16u))") } else { sum };
                self.set(d, x)
            }
            Op::Lop { d, a, b, op, inv_a, inv_b, pout } => {
                let a = self.reg(a);
                let b = self.src(b);
                let a = if inv_a { format!("(~{a})") } else { a };
                let b = if inv_b { format!("(~{b})") } else { b };
                let x = match op {
                    LogicOp::And => format!("({a} & {b})"),
                    LogicOp::Or => format!("({a} | {b})"),
                    LogicOp::Xor => format!("({a} ^ {b})"),
                    LogicOp::PassB => b,
                };
                let mut s = format!("let t = {x}; ");
                s += &self.set(d, "t".into());
                if let Some((p, m)) = pout {
                    let r = match m {
                        LopPred::True => "true",
                        LopPred::Zero => "t == 0u",
                        LopPred::NonZero => "t != 0u",
                    };
                    s += &self.setp(p, r);
                }
                format!("{{ {s} }}")
            }
            Op::Shl { d, a, b, wrap } => {
                let (a, b) = (self.reg(a), self.src(b));
                let x = if wrap { format!("({a} << ({b} & 31u))") } else { format!("select(0u, {a} << ({b} & 31u), {b} < 32u)") };
                self.set(d, x)
            }
            Op::Shr { d, a, b, signed, wrap } => {
                let (a, b) = (self.reg(a), self.src(b));
                let x = match (signed, wrap) {
                    (false, true) => format!("({a} >> ({b} & 31u))"),
                    (false, false) => format!("select(0u, {a} >> ({b} & 31u), {b} < 32u)"),
                    (true, true) => format!("bitcast<u32>(bitcast<i32>({a}) >> ({b} & 31u))"),
                    (true, false) => format!("bitcast<u32>(bitcast<i32>({a}) >> min({b}, 31u))"),
                };
                self.set(d, x)
            }
            Op::Sel { d, a, b, c } => {
                let (a, b) = (self.reg(a), self.src(b));
                let c = self.pred(c);
                self.set(d, format!("select({b}, {a}, {c})"))
            }
            Op::Mov { d, b } => {
                let b = self.src(b);
                self.set(d, b)
            }
            Op::I2f { d, b, dst, src, byte, neg, abs, .. } => {
                let b = self.src(b);
                let v = match src {
                    IntFmt::U32 => format!("f32({b})"),
                    IntFmt::S32 => format!("f32(bitcast<i32>({b}))"),
                    f => format!("f32({})", int_value(&b, f, byte)),
                };
                let v = fmod(v, neg, abs);
                match dst {
                    FloatFmt::F32 => self.setf(d, v),
                    FloatFmt::F16 => self.set(d, format!("pack2x16float(vec2<f32>({v}, 0.0))")),
                }
            }
            Op::F2i { d, b, dst, src, round, neg, abs } => {
                let b = self.src(b);
                let v = match src {
                    FloatFmt::F32 => format!("bitcast<f32>({b})"),
                    FloatFmt::F16 => format!("unpack2x16float({b}).x"),
                };
                let v = fmod(v, neg, abs);
                let v = match round {
                    Round::Rn => format!("round({v})"),
                    Round::Rm => format!("floor({v})"),
                    Round::Rp => format!("ceil({v})"),
                    Round::Rz => format!("trunc({v})"),
                };
                // the hardware saturates to the format's range and turns NaN into 0
                let x = match dst {
                    IntFmt::S32 => format!("bitcast<u32>(i32(clamp({v}, -2147483648.0, 2147483520.0)))"),
                    IntFmt::U32 => format!("u32(clamp({v}, 0.0, 4294967040.0))"),
                    IntFmt::U16 => format!("u32(clamp({v}, 0.0, 65535.0))"),
                    IntFmt::S16 => format!("bitcast<u32>(i32(clamp({v}, -32768.0, 32767.0)))"),
                    IntFmt::U8 => format!("u32(clamp({v}, 0.0, 255.0))"),
                    IntFmt::S8 => format!("bitcast<u32>(i32(clamp({v}, -128.0, 127.0)))"),
                };
                let x = format!("select(0u, {x}, {v} == {v})");
                self.set(d, x)
            }
            Op::F2f { d, b, dst, src, int_round, half, neg, abs, sat: s } => {
                let b = self.src(b);
                let v = match src {
                    FloatFmt::F32 => format!("bitcast<f32>({b})"),
                    FloatFmt::F16 => format!("unpack2x16float({b}).{}", if half { "y" } else { "x" }),
                };
                let v = fmod(v, neg, abs);
                let v = match int_round {
                    None => v,
                    Some(Round::Rn) => format!("round({v})"),
                    Some(Round::Rm) => format!("floor({v})"),
                    Some(Round::Rp) => format!("ceil({v})"),
                    Some(Round::Rz) => format!("trunc({v})"),
                };
                let v = sat(v, s);
                match dst {
                    FloatFmt::F32 => self.setf(d, v),
                    FloatFmt::F16 => self.set(d, format!("pack2x16float(vec2<f32>({v}, 0.0))")),
                }
            }
            Op::I2i { d, b, dst, src, byte, neg, abs, sat: s, cc } => {
                let b = self.src(b);
                let mut v = int_value(&b, src, byte);
                if abs {
                    v = format!("abs({v})");
                }
                if neg {
                    v = format!("(-{v})");
                }
                let unsigned_src = matches!(src, IntFmt::U8 | IntFmt::U16 | IntFmt::U32);
                let x = match (dst, s) {
                    (IntFmt::U32 | IntFmt::S32, false) => format!("bitcast<u32>({v})"),
                    (IntFmt::S32, true) if unsigned_src => format!("min(bitcast<u32>({v}), 0x7fffffffu)"),
                    (IntFmt::S32, true) => format!("bitcast<u32>({v})"),
                    (IntFmt::U32, true) if unsigned_src => format!("bitcast<u32>({v})"),
                    (IntFmt::U32, true) => format!("bitcast<u32>(max({v}, 0))"),
                    (IntFmt::U16, false) => format!("(bitcast<u32>({v}) & 0xffffu)"),
                    (IntFmt::U8, false) => format!("(bitcast<u32>({v}) & 0xffu)"),
                    (IntFmt::S16, false) => format!("bitcast<u32>(extractBits({v}, 0u, 16u))"),
                    (IntFmt::S8, false) => format!("bitcast<u32>(extractBits({v}, 0u, 8u))"),
                    (f, true) => bail!("i2i saturating to {f:?}"),
                };
                let mut s = format!("let t = {x}; ");
                s += &self.set(d, "t".into());
                if cc {
                    self.cc = true;
                    s += " ccz = t == 0u; ccs = bitcast<i32>(t) < 0;";
                }
                format!("{{ {s} }}")
            }
            Op::R2p { a, mask } => {
                let a = self.reg(a);
                let mut s = String::new();
                for i in 0..7 {
                    if mask >> i & 1 != 0 {
                        let _ = write!(s, "p{i} = ({a} & {:#x}u) != 0u; ", 1u32 << i);
                    }
                }
                format!("{{ {s}}}")
            }
            Op::Ald { d, offset, index, vertex, count, output, patch } => {
                if !index.is_zero() || output || patch || !vertex.is_zero() || self.stage != Stage::Vertex {
                    bail!("indexed / output / patch / per-vertex attribute load");
                }
                let mut s = String::new();
                for k in 0..count as u32 {
                    let at = offset + 4 * k;
                    let v = if let Some((n, c)) = Self::generic(at) {
                        self.inputs |= 1 << n;
                        format!("bitcast<u32>(in_a{n}[{c}])")
                    } else if at == 0x2fc {
                        self.vertex_id = true;
                        "in_vertex_id".into()
                    } else {
                        bail!("attribute load from {at:#x}");
                    };
                    s += &self.set(d.plus(k as u8), v);
                }
                s
            }
            Op::Ast { s: src, offset, index, vertex, count } => {
                if !index.is_zero() || !vertex.is_zero() || self.stage != Stage::Vertex {
                    bail!("indexed / per-vertex attribute store");
                }
                let mut s = String::new();
                for k in 0..count as u32 {
                    let at = offset + 4 * k;
                    let v = self.freg(src.plus(k as u8));
                    if (0x70..0x80).contains(&at) {
                        self.position = true;
                        let _ = write!(s, "out_pos[{}] = {v}; ", (at - 0x70) / 4);
                    } else if let Some((n, c)) = Self::generic(at) {
                        self.outputs |= 1 << n;
                        let _ = write!(s, "out_v{n}[{c}] = {v}; ");
                    } else if at == 0x6c {
                        // point size: WGSL draws points one pixel wide
                    } else {
                        bail!("attribute store to {at:#x}");
                    }
                }
                s
            }
            Op::Ldc { d, slot, index, offset, count } => {
                self.cbufs |= 1 << slot;
                let i = self.reg(index);
                let mut s = format!("let at = u32(bitcast<i32>({i}) + {offset}) >> 2u; ");
                for k in 0..count {
                    s += &self.set(d.plus(k), format!("cb{slot}[at + {k}u]"));
                }
                format!("{{ {s} }}")
            }
            Op::Ipa { d, offset, index, mode, mul, sat: s } => {
                if !index.is_zero() || self.stage != Stage::Fragment {
                    bail!("indexed interpolation");
                }
                let v = if let Some((n, c)) = Self::generic(offset) {
                    self.inputs |= 1 << n;
                    format!("in_v{n}[{c}]")
                } else if offset == 0x7c {
                    // 1/w: what the hardware's perspective correction multiplies by (as its reciprocal, `mul`).
                    // The varyings arrive already perspective-correct, so it is 1 and the multiply below is too.
                    "1.0".into()
                } else if (0x70..0x7c).contains(&offset) {
                    format!("in_pos[{}]", (offset - 0x70) / 4)
                } else {
                    bail!("interpolation of {offset:#x}");
                };
                let v = match mode {
                    IpaMode::Multiply => format!("({v} * {})", self.freg(mul)),
                    IpaMode::Pass | IpaMode::Constant => v,
                    IpaMode::Sc => bail!("ipa .sc"),
                };
                self.setf(d, sat(v, s))
            }
            Op::Texs { dst0, dst1, a, b, unit, kind, components } => {
                let (t, smp) = self.texture(unit)?;
                let (u, v, lod) = match kind {
                    TexsKind::Tex2D => (self.freg(a), self.freg(b), None),
                    TexsKind::Tex2DLz => (self.freg(a), self.freg(b), Some("0.0".to_string())),
                    TexsKind::Tex2DLl => (self.freg(a), self.freg(a.plus(1)), Some(self.freg(b))),
                    k => bail!("texs {k:?}"),
                };
                let lod = match (lod, self.stage) {
                    (None, Stage::Vertex) => Some("0.0".to_string()),
                    (l, _) => l,
                };
                let sample = match lod {
                    None => format!("textureSample({t}, {smp}, vec2<f32>({u}, {v}))"),
                    Some(l) => format!("textureSampleLevel({t}, {smp}, vec2<f32>({u}, {v}), {l})"),
                };
                let mut s = format!("let t = {sample}; ");
                let dests = [dst0, dst0.plus(1), dst1, dst1.plus(1)];
                for (k, &c) in components.iter().enumerate() {
                    let d = dests[k];
                    if k >= 2 && dst1.is_zero() {
                        bail!("texs component {k} with no second destination");
                    }
                    s += &self.setf(d, format!("t[{c}]"));
                }
                format!("{{ {s} }}")
            }
            Op::Tmml { d, a, b: _, unit, components } => {
                if self.stage != Stage::Fragment {
                    bail!("tmml outside a fragment stage");
                }
                let (t, _) = self.texture(unit)?;
                self.tmml = true;
                let (u, v) = (self.freg(a), self.freg(a.plus(1)));
                let mut s = format!("let q = query_lod({t}, vec2<f32>({u}, {v})); ");
                let mut k = 0u8;
                for c in 0..2 {
                    if components >> c & 1 != 0 {
                        s += &self.set(d.plus(k), format!("bitcast<u32>(i32(q[{c}] * 256.0))"));
                        k += 1;
                    }
                }
                if components >> 2 != 0 {
                    bail!("tmml components {components:#b}");
                }
                format!("{{ {s} }}")
            }
            Op::Kil => {
                if self.stage != Stage::Fragment {
                    bail!("kil outside a fragment stage");
                }
                "discard;".into()
            }
            Op::Exit => {
                if self.stage == Stage::Fragment {
                    let c: Vec<String> = (0..4).map(|i| self.freg(Reg(i))).collect();
                    format!("out_color = vec4<f32>({}); return;", c.join(", "))
                } else {
                    "return;".into()
                }
            }
            Op::Bra { target } => format!("pc = {target}u; continue;"),
            Op::Sync => {
                let target = sync_target.ok_or_else(|| anyhow!("sync with no target"))?;
                format!("pc = {target}u; continue;")
            }
            Op::Ssy { .. } | Op::Nop | Op::Depbar => String::new(),
        };
        Ok(s)
    }

    fn setp_pair(&self, p: u8, q: u8, r: &str, bop: BoolOp, c: Pred) -> String {
        let c = self.pred(c);
        let mut s = format!("let r = {r}; ");
        s += &self.setp(p, &bool_op(bop, "r", &c));
        s += " ";
        s += &self.setp(q, &bool_op(bop, "!r", &c));
        format!("{{ {s} }}")
    }

    fn run(mut self) -> Result<Shader> {
        let index: BTreeMap<u32, usize> = self.code.iter().enumerate().map(|(i, ins)| (ins.addr, i)).collect();
        let syncs = sync_targets(self.code, &index)?;
        // block leaders: the entry and every jump target
        let mut leaders: BTreeSet<u32> = BTreeSet::new();
        let first = self.code.first().map(|i| i.addr).unwrap_or(8);
        leaders.insert(first);
        for (i, ins) in self.code.iter().enumerate() {
            match ins.op {
                Op::Bra { target } => {
                    leaders.insert(target);
                }
                Op::Sync => {
                    leaders.insert(syncs.get(&i).copied().unwrap_or(first));
                }
                _ => {}
            }
        }
        for &l in &leaders {
            if !index.contains_key(&l) {
                bail!("jump to {l:#x}, not an instruction");
            }
        }
        let structured = leaders.len() > 1;
        let indent = if structured { "      " } else { "  " };
        let mut body = String::new();
        // after an unconditional jump / exit, the code up to the next leader is unreachable
        let mut dead = false;
        for (i, ins) in self.code.iter().enumerate() {
            if structured && leaders.contains(&ins.addr) {
                if ins.addr != first {
                    if !dead {
                        let _ = writeln!(body, "{indent}pc = {}u; continue;", ins.addr);
                    }
                    body += "    }\n";
                }
                let _ = writeln!(body, "    case {}u: {{", ins.addr);
                dead = false;
            }
            if dead || ins.guard.is_false() {
                continue;
            }
            let s = self.instr(ins, syncs.get(&i).copied()).with_context(|| format!("{:#06x} {:?}", ins.addr, ins.op))?;
            if s.is_empty() {
                continue;
            }
            let g = self.pred(ins.guard);
            if g == "true" {
                let _ = writeln!(body, "{indent}{s}");
                if matches!(ins.op, Op::Bra { .. } | Op::Sync | Op::Exit) {
                    dead = true;
                }
            } else {
                let _ = writeln!(body, "{indent}if ({g}) {{ {s} }}");
            }
        }
        if structured {
            if !dead {
                let _ = writeln!(body, "{indent}return;");
            }
            body += "    }\n";
        }
        self.body = body;
        Ok(self.finish(structured))
    }

    fn finish(self, structured: bool) -> Shader {
        let mut w = String::new();
        let stage = self.stage;
        let base = if stage == Stage::Vertex { 0 } else { FRAGMENT_CBUF_BASE };
        let tbase = if stage == Stage::Vertex { VERTEX_TEXTURE_BASE } else { 0 };
        w += "// Translated by Effect Forge (src/draw/mx) from the game's Maxwell code.\n";
        for n in 0..32 {
            if self.cbufs >> n & 1 != 0 {
                let _ = writeln!(w, "@group(0) @binding({}) var<storage, read> cb{n}: array<u32>;", base + n);
            }
        }
        for &u in &self.units {
            let _ = writeln!(w, "@group(1) @binding({}) var tex{u}: texture_2d<f32>;", tbase + u);
            let _ = writeln!(w, "@group(1) @binding({}) var smp{u}: sampler;", tbase + u + SAMPLER_SPLIT);
        }
        w += "fn fmz(a: f32, b: f32) -> f32 { return select(a * b, 0.0, a == 0.0 || b == 0.0); }\n";
        if self.tmml {
            w += "fn query_lod(t: texture_2d<f32>, uv: vec2<f32>) -> vec2<f32> {\n";
            w += "  let size = vec2<f32>(textureDimensions(t, 0));\n";
            w += "  let dx = dpdx(uv * size); let dy = dpdy(uv * size);\n";
            w += "  let lod = 0.5 * log2(max(dot(dx, dx), dot(dy, dy)));\n";
            w += "  return vec2<f32>(clamp(lod, 0.0, max(f32(textureNumLevels(t)) - 1.0, 0.0)), lod);\n}\n";
        }
        let outputs = if stage == Stage::Vertex { self.outputs | self.extra_outputs } else { 0 };
        match stage {
            Stage::Vertex => {
                for n in 0..32 {
                    if self.inputs >> n & 1 != 0 {
                        let _ = writeln!(w, "var<private> in_a{n}: vec4<f32>;");
                    }
                }
                if self.vertex_id {
                    w += "var<private> in_vertex_id: u32;\n";
                }
                w += "var<private> out_pos: vec4<f32>;\n";
                for n in 0..32 {
                    if outputs >> n & 1 != 0 {
                        let _ = writeln!(w, "var<private> out_v{n}: vec4<f32> = vec4<f32>(0.0, 0.0, 0.0, 1.0);");
                    }
                }
                w += "struct VertexOut {\n  @builtin(position) pos: vec4<f32>,\n";
                for n in 0..32 {
                    if outputs >> n & 1 != 0 {
                        let _ = writeln!(w, "  @location({n}) v{n}: vec4<f32>,");
                    }
                }
                w += "};\n";
            }
            Stage::Fragment => {
                let _ = writeln!(
                    w,
                    "struct AlphaTest {{ func: u32, reference: f32, point_size: f32, pad: u32 }};\n\
                     @group(0) @binding({ALPHA_TEST_BINDING}) var<storage, read> alpha_test: AlphaTest;"
                );
                for n in 0..32 {
                    if self.inputs >> n & 1 != 0 {
                        let _ = writeln!(w, "var<private> in_v{n}: vec4<f32>;");
                    }
                }
                w += "var<private> in_pos: vec4<f32>;\nvar<private> out_color: vec4<f32>;\n";
                // 0 or >= 8 always, 1 never, then <, ==, <=, >, !=, >= against the reference
                w += "fn alpha_passes(a: f32) -> bool {\n  let r = alpha_test.reference;\n  switch alpha_test.func {\n";
                w += "    case 1u: { return false; }\n    case 2u: { return a < r; }\n    case 3u: { return a == r; }\n";
                w += "    case 4u: { return a <= r; }\n    case 5u: { return a > r; }\n    case 6u: { return a != r; }\n";
                w += "    case 7u: { return a >= r; }\n    default: { return true; }\n  }\n}\n";
            }
        }
        w += "fn run() {\n";
        for r in 0..=self.max_reg {
            let _ = writeln!(w, "  var r{r}: u32 = 0u;");
        }
        for p in 0..7 {
            let _ = writeln!(w, "  var p{p}: bool = false;");
        }
        if self.cc {
            w += "  var ccz: bool = false;\n  var ccs: bool = false;\n";
        }
        if structured {
            let first = self.code.first().map(|i| i.addr).unwrap_or(8);
            let _ = writeln!(w, "  var pc: u32 = {first}u;\n  loop {{\n    switch pc {{");
            w += &self.body;
            w += "    default: { return; }\n    }\n  }\n";
        } else {
            w += &self.body;
        }
        w += "}\n";
        match stage {
            Stage::Vertex => {
                let mut params = Vec::new();
                let mut copy = String::new();
                for n in 0..32 {
                    if self.inputs >> n & 1 != 0 {
                        params.push(format!("@location({n}) a{n}: vec4<f32>"));
                        let _ = writeln!(copy, "  in_a{n} = a{n};");
                    }
                }
                if self.vertex_id {
                    params.push("@builtin(vertex_index) vertex_id: u32".into());
                    copy += "  in_vertex_id = vertex_id;\n";
                }
                let _ = writeln!(w, "@vertex\nfn main({}) -> VertexOut {{", params.join(", "));
                w += &copy;
                w += "  run();\n  var o: VertexOut;\n  o.pos = out_pos;\n";
                for n in 0..32 {
                    if outputs >> n & 1 != 0 {
                        let _ = writeln!(w, "  o.v{n} = out_v{n};");
                    }
                }
                w += "  return o;\n}\n";
            }
            Stage::Fragment => {
                let mut params = vec!["@builtin(position) pos: vec4<f32>".to_string()];
                let mut copy = "  in_pos = pos;\n".to_string();
                for n in 0..32 {
                    if self.inputs >> n & 1 != 0 {
                        params.push(format!("@location({n}) v{n}: vec4<f32>"));
                        let _ = writeln!(copy, "  in_v{n} = v{n};");
                    }
                }
                let _ = writeln!(w, "@fragment\nfn main({}) -> @location(0) vec4<f32> {{", params.join(", "));
                w += &copy;
                w += "  run();\n  if (!alpha_passes(out_color.a)) { discard; }\n  return out_color;\n}\n";
            }
        }
        Shader { wgsl: w, cbufs: self.cbufs, inputs: self.inputs, outputs, units: self.units.into_iter().collect() }
    }
}
