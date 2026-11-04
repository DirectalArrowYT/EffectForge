//! Print the byte offset of every emitter field, by poking each byte and seeing which parsed
//! field moves.
//!
//! The runtime reads emitter data by raw offset; the parser reads it by name. This joins the
//! two without hand-counting a version-dependent layout: offsets printed here are relative to
//! the start of the EMTR section's binary data.
//!
//! Usage: cargo run --release --example emitter_offsets -- <eff> [emitter-index]

use effect_library::EmitterData;
use serde_json::Value;
use std::io::Cursor;

fn flatten(prefix: &str, v: &Value, out: &mut Vec<(String, String)>) {
    match v {
        Value::Object(m) => {
            for (k, c) in m {
                flatten(&format!("{prefix}.{k}"), c, out);
            }
        }
        Value::Array(a) => {
            for (i, c) in a.iter().enumerate() {
                flatten(&format!("{prefix}[{i}]"), c, out);
            }
        }
        other => out.push((prefix.to_string(), other.to_string())),
    }
}

fn parse(bytes: &[u8], version: u16) -> Option<Vec<(String, String)>> {
    let data = EmitterData::read(&mut Cursor::new(bytes), version).ok()?;
    let mut out = Vec::new();
    flatten("", &serde_json::to_value(&data).ok()?, &mut out);
    Some(out)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: <eff> [emitter-index]");
    let index: usize = args.next().map(|s| s.parse().expect("index")).unwrap_or(0);
    let file = std::fs::read(&path).expect("read eff");

    let vfxb = file.windows(4).position(|w| w == b"VFXB").expect("no VFXB");
    let version = u16::from_le_bytes([file[vfxb + 0x0a], file[vfxb + 0x0b]]);
    let u32_at = |o: usize| u32::from_le_bytes(file[o..o + 4].try_into().unwrap());
    let start = file
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"EMTR")
        .map(|(i, _)| i)
        .nth(index)
        .expect("no such EMTR");
    let (size, attr, binary) = (u32_at(start + 4), u32_at(start + 0x10), u32_at(start + 0x14));
    let end = if attr != u32::MAX { attr } else { size };
    let mut bytes = file[start + binary as usize..start + end as usize].to_vec();
    println!("# vfx version {version}, EMTR at {start:#x}, binary data at +{binary:#x}, {:#x} bytes", bytes.len());

    let base = parse(&bytes, version).expect("baseline parse");
    let mut last = String::new();
    for offset in 0..bytes.len() {
        let saved = bytes[offset];
        let mut names: Vec<String> = Vec::new();
        for flip in [0x01u8, 0x80, 0xff] {
            bytes[offset] = saved ^ flip;
            if let Some(now) = parse(&bytes, version) {
                if now.len() == base.len() {
                    for (a, b) in base.iter().zip(&now) {
                        if a.1 != b.1 && !names.contains(&a.0) {
                            names.push(a.0.clone());
                        }
                    }
                }
            }
        }
        bytes[offset] = saved;
        let label = names.join(" ");
        if !label.is_empty() && label != last {
            println!("{offset:#06x} {label}");
        }
        if !label.is_empty() {
            last = label;
        }
    }
}
