//! A decoded stage as a WGSL module for wgpu.
//!
//! The translation is literal: registers are `u32` variables, predicates `bool`s, and each instruction becomes a
//! statement or two (`emit`). A program that branches is cut into blocks at every jump target (`flow`) and runs as
//! `loop { switch pc { ... } }`.
//!
//! The binding layout, which `draw::gpu` follows:
//! * group 0: constant buffer `c<n>` at binding n for the vertex stage and 20 + n for the fragment stage, as a
//!   read-only storage array of `u32`; the fragment stage's alpha test block (func, reference, point size, pad) at 39.
//! * group 1: texture unit u at binding u for the fragment stage and 32 + u for the vertex stage, its sampler 64
//!   bindings above that.
//! * vertex attribute n at location n as a `vec4<f32>`; varying n at location n.
//! * the fragment stage writes render target 0 at location 0.

use std::fmt::Write as _;

use anyhow::{anyhow, bail, Context, Result};

use super::decode::decode_program;
use super::emit::{Usage, Writer};
use super::flow::Flow;
use super::isa::{Instr, Op};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    Vertex,
    Fragment,
}

pub const FRAGMENT_CBUF_BASE: u32 = 20;
pub const VERTEX_TEXTURE_BASE: u32 = 32;
pub const SAMPLER_SPLIT: u32 = 64;
pub const ALPHA_TEST_BINDING: u32 = 39;

/// Size of the program header in front of the code.
const HEADER: usize = 0x50;

/// One translated stage and what a draw has to give it.
#[derive(Clone, Debug)]
pub struct Shader {
    pub wgsl: String,
    /// Bit n: reads constant buffer c<n>.
    pub cbufs: u32,
    /// Bit n: reads generic input n (vertex attribute / varying).
    pub inputs: u32,
    /// Texture units sampled, ascending.
    pub units: Vec<u32>,
}

/// The program (header and code) in an NVN code section, `code`, whose control section is `ctl`: the program starts
/// at 0x30 and the control section holds its length at 0x6f8.
pub fn program<'a>(code: &'a [u8], ctl: &[u8]) -> Result<&'a [u8]> {
    let len = ctl.get(0x6f8..0x6fc).ok_or_else(|| anyhow!("control section too short"))?;
    let len = u32::from_le_bytes(len.try_into().unwrap()) as usize;
    code.get(0x30..0x30 + len).filter(|p| p.len() >= HEADER).ok_or_else(|| anyhow!("code section too short"))
}

/// Translate one stage. A vertex stage also writes every varying in `extra_outputs`, as (0, 0, 0, 1) where its code
/// does not, so it can feed a fragment stage that reads more than it writes.
pub fn translate(program: &[u8], stage: Stage, extra_outputs: u32) -> Result<Shader> {
    let word = |i: usize| u32::from_le_bytes(program[4 * i..4 * i + 4].try_into().unwrap());
    let kind = word(0) >> 10 & 15;
    if kind != if stage == Stage::Vertex { 1 } else { 5 } {
        bail!("the program header says shader type {kind}");
    }
    // only render target 0, all four channels
    if stage == Stage::Fragment && (word(18) != 0xf || word(19) & 3 != 0) {
        bail!("fragment outputs {:#x} / {:#x}", word(18), word(19));
    }
    let code = decode_program(&program[HEADER..])?;
    let flow = Flow::of(&code)?;
    let mut writer = Writer::new(stage);
    let body = body(&code, &flow, &mut writer)?;
    let used = writer.used;
    let outputs = if stage == Stage::Vertex { used.outputs | extra_outputs } else { 0 };
    let wgsl = module(stage, &used, outputs, &flow, &body);
    Ok(Shader { wgsl, cbufs: used.cbufs, inputs: used.inputs, units: used.units.into_iter().collect() })
}

/// The statements of `run()`: straight, or as the cases of the dispatch switch when the program branches.
fn body(code: &[Instr], flow: &Flow, writer: &mut Writer) -> Result<String> {
    let blocks = flow.branches();
    let indent = if blocks { "      " } else { "  " };
    let mut out = String::new();
    // code after an unconditional jump or exit, up to the next block, never runs
    let mut unreachable = false;
    for (i, ins) in code.iter().enumerate() {
        if blocks && flow.leaders.contains(&ins.addr) {
            if ins.addr != flow.entry {
                if !unreachable {
                    let _ = writeln!(out, "{indent}pc = {}u; continue;", ins.addr);
                }
                out += "    }\n";
            }
            let _ = writeln!(out, "    case {}u: {{", ins.addr);
            unreachable = false;
        }
        if unreachable || ins.guard.never() {
            continue;
        }
        let s = writer
            .statement(ins.op, flow.sync_target.get(&i).copied())
            .with_context(|| format!("{:#06x} {:?}", ins.addr, ins.op))?;
        if s.is_empty() {
            continue;
        }
        let guard = writer.pred(ins.guard);
        if guard == "true" {
            let _ = writeln!(out, "{indent}{s}");
            unreachable = matches!(ins.op, Op::Bra { .. } | Op::Sync | Op::Exit);
        } else {
            let _ = writeln!(out, "{indent}if ({guard}) {{ {s} }}");
        }
    }
    if blocks {
        if !unreachable {
            let _ = writeln!(out, "{indent}return;");
        }
        out += "    }\n";
    }
    Ok(out)
}

/// The bits set in `mask`, ascending.
fn bits(mask: u32) -> impl Iterator<Item = u32> {
    (0..32).filter(move |n| mask >> n & 1 != 0)
}

const FMZ: &str = "fn fmz(a: f32, b: f32) -> f32 { return select(a * b, 0.0, a == 0.0 || b == 0.0); }\n";

/// TMML's result as (clamped level, raw level), from the texture's size and the screen derivatives.
const QUERY_LOD: &str = "fn query_lod(t: texture_2d<f32>, uv: vec2<f32>) -> vec2<f32> {
  let size = vec2<f32>(textureDimensions(t, 0));
  let dx = dpdx(uv * size); let dy = dpdy(uv * size);
  let lod = 0.5 * log2(max(dot(dx, dx), dot(dy, dy)));
  return vec2<f32>(clamp(lod, 0.0, max(f32(textureNumLevels(t)) - 1.0, 0.0)), lod);
}
";

/// The game's alpha test: 0 or 8 and up always pass, 1 never, 2..7 compare against the reference by
/// <, ==, <=, >, !=, >=.
const ALPHA_PASSES: &str = "fn alpha_passes(a: f32) -> bool {
  let r = alpha_test.reference;
  switch alpha_test.func {
    case 1u: { return false; }
    case 2u: { return a < r; }
    case 3u: { return a == r; }
    case 4u: { return a <= r; }
    case 5u: { return a > r; }
    case 6u: { return a != r; }
    case 7u: { return a >= r; }
    default: { return true; }
  }
}
";

fn module(stage: Stage, used: &Usage, outputs: u32, flow: &Flow, body: &str) -> String {
    let vertex = stage == Stage::Vertex;
    let (cbuf_base, texture_base) = if vertex { (0, VERTEX_TEXTURE_BASE) } else { (FRAGMENT_CBUF_BASE, 0) };
    let mut w = String::from("// Translated by Effect Forge (src/draw/mx) from the game's Maxwell code.\n");
    for n in bits(used.cbufs) {
        let _ = writeln!(w, "@group(0) @binding({}) var<storage, read> cb{n}: array<u32>;", cbuf_base + n);
    }
    for &u in &used.units {
        let _ = writeln!(w, "@group(1) @binding({}) var tex{u}: texture_2d<f32>;", texture_base + u);
        let _ = writeln!(w, "@group(1) @binding({}) var smp{u}: sampler;", texture_base + u + SAMPLER_SPLIT);
    }
    w += FMZ;
    if used.query_lod {
        w += QUERY_LOD;
    }

    // the stage's inputs and outputs, as module variables run() reads and writes
    if vertex {
        for n in bits(used.inputs) {
            let _ = writeln!(w, "var<private> in_a{n}: vec4<f32>;");
        }
        if used.vertex_id {
            w += "var<private> in_vertex_id: u32;\n";
        }
        w += "var<private> out_pos: vec4<f32>;\n";
        for n in bits(outputs) {
            let _ = writeln!(w, "var<private> out_v{n}: vec4<f32> = vec4<f32>(0.0, 0.0, 0.0, 1.0);");
        }
        w += "struct VertexOut {\n  @builtin(position) pos: vec4<f32>,\n";
        for n in bits(outputs) {
            let _ = writeln!(w, "  @location({n}) v{n}: vec4<f32>,");
        }
        w += "};\n";
    } else {
        w += "struct AlphaTest { func: u32, reference: f32, point_size: f32, pad: u32 };\n";
        let _ = writeln!(w, "@group(0) @binding({ALPHA_TEST_BINDING}) var<storage, read> alpha_test: AlphaTest;");
        for n in bits(used.inputs) {
            let _ = writeln!(w, "var<private> in_v{n}: vec4<f32>;");
        }
        w += "var<private> in_pos: vec4<f32>;\nvar<private> out_color: vec4<f32>;\n";
        w += ALPHA_PASSES;
    }

    w += "fn run() {\n";
    for r in 0..=used.max_reg {
        let _ = writeln!(w, "  var r{r}: u32 = 0u;");
    }
    for p in 0..7 {
        let _ = writeln!(w, "  var p{p}: bool = false;");
    }
    if used.cc {
        w += "  var ccz: bool = false;\n  var ccs: bool = false;\n";
    }
    if flow.branches() {
        let _ = writeln!(w, "  var pc: u32 = {}u;\n  loop {{\n    switch pc {{", flow.entry);
        w += body;
        w += "    default: { return; }\n    }\n  }\n";
    } else {
        w += body;
    }
    w += "}\n";

    // the entry point copies the inputs in, runs, and hands the outputs back
    let mut params = Vec::new();
    let mut copy = String::new();
    if vertex {
        for n in bits(used.inputs) {
            params.push(format!("@location({n}) a{n}: vec4<f32>"));
            let _ = writeln!(copy, "  in_a{n} = a{n};");
        }
        if used.vertex_id {
            params.push("@builtin(vertex_index) vertex_id: u32".into());
            copy += "  in_vertex_id = vertex_id;\n";
        }
        let _ = writeln!(w, "@vertex\nfn main({}) -> VertexOut {{", params.join(", "));
        w += &copy;
        w += "  run();\n  var o: VertexOut;\n  o.pos = out_pos;\n";
        for n in bits(outputs) {
            let _ = writeln!(w, "  o.v{n} = out_v{n};");
        }
        w += "  return o;\n}\n";
    } else {
        params.push("@builtin(position) pos: vec4<f32>".to_string());
        copy += "  in_pos = pos;\n";
        for n in bits(used.inputs) {
            params.push(format!("@location({n}) v{n}: vec4<f32>"));
            let _ = writeln!(copy, "  in_v{n} = v{n};");
        }
        let _ = writeln!(w, "@fragment\nfn main({}) -> @location(0) vec4<f32> {{", params.join(", "));
        w += &copy;
        w += "  run();\n  if (!alpha_passes(out_color.a)) { discard; }\n  return out_color;\n}\n";
    }
    w
}
