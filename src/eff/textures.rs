//! The texture pool: reading a texture out of it, and putting the user's own pictures into it.
//!
//! An `.eff` keeps its textures in one BNTX. `effect_library` cuts a texture out of it as a BNTX of its own and puts
//! a pool back together from such pieces, around the original pool so its alignment, name and string order stay as
//! they were. The `bntx` crate turns one of those pieces into an `image_dds` surface (unswizzled) and builds one from
//! a surface. Forge goes between pictures and pool through those two.
//!
//! A texture's header (BRTI) also says which stored channel each sampled channel reads, and that is part of what the
//! texture is: a BC4 mask stores one channel and has all four read it, so a shader that samples alpha gets the
//! shape. Forge reads with it, and an import keeps the original's. Formats `bntx` does not know (0x0701 in the game's
//! files) and signed ones (8-bit RGBA can't hold their negative half) are reported, not converted.

use std::io::Cursor;

use anyhow::{anyhow, bail, Context, Result};
use binrw::BinRead;
use bntx::{Bntx, SurfaceFormat};
use image::{Rgba, RgbaImage};
use image_dds::{ImageFormat, Mipmaps, Quality, SurfaceRgba8};

/// A pool texture's size, mip count and format.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureDesc {
    pub width: u32,
    pub height: u32,
    pub mipmaps: u32,
    /// The format as `bntx` names it (`BC7Srgb`), or `unknown (0x…)`.
    pub format: String,
    /// Whether Forge can decode it and encode it back.
    pub convertible: bool,
}

/// The BRTI header fields Forge reads.
struct Header {
    format_code: u32,
    mipmaps: u16,
    width: u32,
    height: u32,
    /// Per sampled channel (R, G, B, A) the source: 0 zero, 1 one, 2..5 stored red..alpha.
    channels: [u8; 4],
}

const IDENTITY: [u8; 4] = [2, 3, 4, 5];

impl Header {
    fn read(pool: &[u8], index: usize) -> Result<Self> {
        let at = |offset: usize, len: usize| pool.get(offset..offset + len).ok_or_else(|| anyhow!("BNTX truncated at {offset:#x}"));
        let u32_at = |offset| Ok::<_, anyhow::Error>(u32::from_le_bytes(at(offset, 4)?.try_into().unwrap()));
        let u64_at = |offset| Ok::<_, anyhow::Error>(u64::from_le_bytes(at(offset, 8)?.try_into().unwrap()) as usize);
        let count = u32_at(0x24)? as usize;
        if index >= count {
            bail!("texture index {index} is outside a pool of {count}");
        }
        let brti = u64_at(u64_at(0x28)? + index * 8)?;
        if pool.get(brti..brti + 4) != Some(b"BRTI") {
            bail!("texture {index} does not start with a BRTI block");
        }
        Ok(Self {
            mipmaps: u16::from_le_bytes(at(brti + 0x16, 2)?.try_into().unwrap()),
            format_code: u32_at(brti + 0x1c)?,
            width: u32_at(brti + 0x24)?,
            height: u32_at(brti + 0x28)?,
            channels: u32_at(brti + 0x58)?.to_le_bytes(),
        })
    }

    fn format(&self) -> Option<SurfaceFormat> {
        SurfaceFormat::read_le(&mut Cursor::new(self.format_code.to_le_bytes())).ok()
    }
}

fn is_signed(format: ImageFormat) -> bool {
    use ImageFormat::*;
    matches!(format, R8Snorm | Rg8Snorm | Rgba8Snorm | BC4RSnorm | BC5RgSnorm | R16Snorm | Rg16Snorm | Rgba16Snorm)
}

/// Texture `index` of the pool. Reads the header itself rather than through `bntx`, which refuses a whole texture
/// whose format it does not know.
pub fn describe(pool: &[u8], index: usize, _name: &str) -> Result<TextureDesc> {
    let header = Header::read(pool, index)?;
    let format = header.format();
    Ok(TextureDesc {
        width: header.width,
        height: header.height,
        mipmaps: header.mipmaps as u32,
        format: format.map_or_else(|| format!("unknown ({:#06x})", header.format_code), |f| format!("{f:?}")),
        convertible: format.and_then(|f| ImageFormat::try_from(f).ok()).is_some_and(|f| !is_signed(f)),
    })
}

/// Texture `index` decoded to RGBA8, every mip, as stored (no channel mapping).
fn decode(pool: &[u8], index: usize, name: &str) -> Result<SurfaceRgba8<Vec<u8>>> {
    let desc = describe(pool, index, name)?;
    if !desc.convertible {
        bail!("'{name}' is {} — Effect Forge cannot convert that format", desc.format);
    }
    let single = cut(pool, index, name)?;
    let bntx = Bntx::read_le(&mut Cursor::new(&single)).map_err(|e| anyhow!("bntx could not parse '{name}': {e}"))?;
    let surface = bntx.to_surface().map_err(|e| anyhow!("cannot read '{name}' as an image: {e}"))?;
    surface.decode_rgba8().map_err(|e| anyhow!("cannot decode '{name}' ({}): {e}", desc.format))
}

fn level(surface: &SurfaceRgba8<Vec<u8>>, level: u32, name: &str) -> Result<RgbaImage> {
    surface.to_image(level).map_err(|e| anyhow!("cannot build mip {level} of '{name}': {e}"))
}

fn cut(pool: &[u8], index: usize, name: &str) -> Result<Vec<u8>> {
    effect_library::bntx::build_single_texture_bntx_public(pool, index, name)
        .with_context(|| format!("slicing texture '{name}' out of the pool"))
}

/// Stored pixels as sampled through `channels`. `other` is what a source past 5 gives, from the stored pixel and the
/// channel being filled.
fn sampled(image: &mut RgbaImage, channels: [u8; 4], other: fn(&[u8; 4], usize) -> u8) {
    if channels == IDENTITY {
        return;
    }
    for px in image.pixels_mut() {
        let stored = px.0;
        for (c, source) in channels.into_iter().enumerate() {
            px.0[c] = match source {
                0 => 0,
                1 => 255,
                2..=5 => stored[source as usize - 2],
                _ => other(&stored, c),
            };
        }
    }
}

/// A texture as a shader samples it: every mip, channels mapped, and whether the sampler linearises it (sRGB).
pub struct SampledTexture {
    pub levels: Vec<RgbaImage>,
    pub srgb: bool,
}

pub fn decode_for_sampling(pool: &[u8], index: usize, name: &str) -> Result<SampledTexture> {
    let surface = decode(pool, index, name)?;
    let header = Header::read(pool, index)?;
    let levels = (0..surface.mipmaps.max(1))
        .map(|n| {
            let mut image = level(&surface, n, name)?;
            sampled(&mut image, header.channels, |_, _| 255);
            Ok(image)
        })
        .collect::<Result<_>>()?;
    Ok(SampledTexture { levels, srgb: header.format().is_some_and(|f| format!("{f:?}").contains("Srgb")) })
}

/// Which picture of a texture: as the user edits it, or as stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Form {
    /// As sampled. On import, a texture that holds one thing (a shape, or a colour with a shape) takes that thing
    /// from the picture however it was painted; black and see-through both mean nothing drawn.
    #[default]
    Editable,
    /// The stored channels exactly.
    Raw,
}

/// Texture `index`'s top mip in `form`, its longest side brought down to `max_edge`.
pub fn decode_preview(pool: &[u8], index: usize, name: &str, form: Form, max_edge: Option<u32>) -> Result<RgbaImage> {
    let mut image = level(&decode(pool, index, name)?, 0, name)?;
    if form == Form::Editable {
        let channels = Header::read(pool, index).map_or(IDENTITY, |h| h.channels);
        sampled(&mut image, channels, |stored, c| stored[c]);
    }
    Ok(match max_edge {
        Some(edge) if image.width() > edge || image.height() > edge => {
            let scale = edge as f32 / image.width().max(image.height()) as f32;
            let side = |n: u32| ((n as f32 * scale).round() as u32).max(1);
            image::imageops::thumbnail(&image, side(image.width()), side(image.height()))
        }
        _ => image,
    })
}

/// What a texture holds once sampled.
#[derive(Clone, Copy)]
enum Holds {
    /// Brightness and alpha are one value (a BC4 mask).
    Shape,
    /// One colour, with the shape in alpha.
    ColourAndShape([u8; 3]),
    /// No transparency.
    Opaque,
    /// Colour and alpha both vary on their own.
    Free,
}

fn brightness(px: &Rgba<u8>) -> u8 {
    ((px[0] as u32 * 299 + px[1] as u32 * 587 + px[2] as u32 * 114) / 1000).min(255) as u8
}

impl Holds {
    fn of(image: &RgbaImage) -> Self {
        let (mut lo, mut hi) = ([255u8; 3], [0u8; 3]);
        let (mut opaque, mut shape) = (true, true);
        for px in image.pixels() {
            for c in 0..3 {
                (lo[c], hi[c]) = (lo[c].min(px[c]), hi[c].max(px[c]));
            }
            opaque &= px[3] == 255;
            // Within 6 levels counts as the same value.
            shape &= brightness(px).abs_diff(px[3]) <= 6;
        }
        if opaque {
            Self::Opaque
        } else if shape {
            Self::Shape
        // BCn gives a flat colour back within a level or two.
        } else if (0..3).all(|c| hi[c] - lo[c] <= 4) {
            Self::ColourAndShape([0, 1, 2].map(|c| ((lo[c] as u16 + hi[c] as u16) / 2) as u8))
        } else {
            Self::Free
        }
    }

    /// The painted picture as it should sample. A shape is whichever of brightness and alpha is less, so painting
    /// black and erasing both remove it, and a paint program that flattened alpha or matted onto white still works.
    fn from_painted(self, image: &mut RgbaImage) {
        for px in image.pixels_mut() {
            let shape = brightness(px).min(px[3]);
            match self {
                Self::Shape => *px = Rgba([shape; 4]),
                Self::ColourAndShape([r, g, b]) => *px = Rgba([r, g, b, shape]),
                Self::Opaque => px[3] = 255,
                Self::Free => {}
            }
        }
    }
}

/// Sampled pixels back to stored channels. A stored channel nothing samples gets 0, or 255 for alpha.
fn stored(image: &mut RgbaImage, channels: [u8; 4]) {
    let read_by = [0, 1, 2, 3].map(|s| channels.iter().position(|&c| c as usize == s + 2));
    for px in image.pixels_mut() {
        let sampled = px.0;
        px.0 = [0, 1, 2, 3].map(|s| read_by[s].map_or(if s == 3 { 255 } else { 0 }, |c| sampled[c]));
    }
}

/// What an import encoded to.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportReport {
    pub format: String,
    /// The original format, when it could not be encoded and BC7 sRGB was used instead.
    pub format_substituted_from: Option<String>,
}

/// `png` encoded as a single-texture BNTX named `output_name`, in texture `template`'s format, size and channel
/// mapping. The size must match: emitters' UV settings were made for it.
fn encode_like(pool: &[u8], names: &[String], template: usize, output_name: &str, png: &[u8], form: Form) -> Result<(Vec<u8>, ImportReport)> {
    let template_name = names.get(template).ok_or_else(|| anyhow!("texture index {template} is outside a pool of {}", names.len()))?;
    let desc = describe(pool, template, template_name)?;
    let header = Header::read(pool, template)?;
    let (target, substituted) = match header.format().map(ImageFormat::try_from) {
        // BC6h encodes from floats, which a PNG doesn't have.
        Some(Ok(f)) if !matches!(f, ImageFormat::BC6hRgbUfloat | ImageFormat::BC6hRgbSfloat) => (f, None),
        _ => (ImageFormat::BC7RgbaUnormSrgb, Some(desc.format.clone())),
    };

    let mut image = image::load_from_memory_with_format(png, image::ImageFormat::Png).context("reading the PNG")?.to_rgba8();
    if image.width() == 0 || image.height() == 0 {
        bail!("the PNG is empty");
    }
    if (image.width(), image.height()) != (desc.width, desc.height) {
        bail!(
            "that image is {}×{}, but '{template_name}' is {}×{} — effect textures have to keep their size, because \
             emitters carry UV rects authored against it. Resize the image and try again.",
            image.width(),
            image.height(),
            desc.width,
            desc.height
        );
    }
    if form == Form::Editable {
        // A template that won't decode is taken as stored.
        if let Some(mut original) = decode(pool, template, template_name).ok().and_then(|s| level(&s, 0, template_name).ok()) {
            sampled(&mut original, header.channels, |stored, c| stored[c]);
            Holds::of(&original).from_painted(&mut image);
            stored(&mut image, header.channels);
        }
    }

    let encoded = SurfaceRgba8::from_image(&image)
        .encode(target, Quality::Normal, Mipmaps::GeneratedAutomatic)
        .map_err(|e| anyhow!("encoding the PNG as {target:?}: {e}"))?;
    let mut built = Bntx::from_surface(encoded, output_name).map_err(|e| anyhow!("building a BNTX for '{output_name}': {e}"))?;
    // `bntx` writes the identity mapping, right for a BC7 stand-in but not for a mask kept in its own format.
    if let (None, Some(brti)) = (&substituted, built.nx_header.brtis.first_mut()) {
        brti.brti.comp_sel = u32::from_le_bytes(header.channels);
    }
    let mut bytes = Cursor::new(Vec::new());
    built.write(&mut bytes).with_context(|| format!("writing the BNTX for '{output_name}'"))?;
    Ok((bytes.into_inner(), ImportReport { format: format!("{target:?}"), format_substituted_from: substituted }))
}

/// The pool rebuilt with `slots` in order: `Some(i)` keeps texture `i` as is, `None` takes the next of `added`.
fn rebuild(pool: &[u8], names: &[String], slots: &[Option<usize>], added: Vec<Vec<u8>>) -> Result<Vec<u8>> {
    let mut added = added.into_iter();
    let pieces = slots
        .iter()
        .map(|slot| match slot {
            Some(i) => cut(pool, *i, &names[*i]),
            None => added.next().ok_or_else(|| anyhow!("internal: fewer new textures than pool slots")),
        })
        .collect::<Result<Vec<_>>>()?;
    effect_library::bntx::rebuild_from_base_and_exports(pool, &pieces).context("rebuilding the texture pool")
}

fn check_index(names: &[String], index: usize) -> Result<()> {
    if index >= names.len() {
        bail!("texture index {index} is outside a pool of {}", names.len());
    }
    Ok(())
}

fn check_new_name(names: &[String], name: &str) -> Result<()> {
    if names.iter().any(|n| n == name) {
        bail!("this eff already holds a texture called '{name}'");
    }
    Ok(())
}

fn all_and_one_more(names: &[String]) -> Vec<Option<usize>> {
    (0..names.len()).map(Some).chain([None]).collect()
}

/// The pool with texture `index` replaced by `png`, in the format it had. `names` is the pool's texture names in
/// order. The report says when that format could not be encoded and another was used.
pub fn replace_with_png(pool: &[u8], names: &[String], index: usize, png: &[u8], form: Form) -> Result<(Vec<u8>, ImportReport)> {
    check_index(names, index)?;
    let (replacement, report) = encode_like(pool, names, index, &names[index], png, form)?;
    let slots: Vec<_> = (0..names.len()).map(|i| (i != index).then_some(i)).collect();
    Ok((rebuild(pool, names, &slots, vec![replacement])?, report))
}

/// The pool with a byte-for-byte copy of texture `src` added as `new_name`: a texture of the effect's own to change.
pub fn duplicate_texture(pool: &[u8], names: &[String], src: usize, new_name: &str) -> Result<Vec<u8>> {
    check_index(names, src)?;
    check_new_name(names, new_name)?;
    rebuild(pool, names, &all_and_one_more(names), vec![cut(pool, src, new_name)?])
}

/// The pool with `png` added as `new_name`, made like texture `template`.
pub fn add_texture_from_png(
    pool: &[u8],
    names: &[String],
    template: usize,
    new_name: &str,
    png: &[u8],
    form: Form,
) -> Result<(Vec<u8>, ImportReport)> {
    check_new_name(names, new_name)?;
    let (built, report) = encode_like(pool, names, template, new_name, png, form)?;
    Ok((rebuild(pool, names, &all_and_one_more(names), vec![built])?, report))
}

/// The pool without texture `index`. Whether anything still samples it is the caller's check.
pub fn remove_texture(pool: &[u8], names: &[String], index: usize) -> Result<Vec<u8>> {
    check_index(names, index)?;
    if names.len() == 1 {
        bail!("'{}' is the only texture in this eff, and a pool can't be empty", names[index]);
    }
    let slots: Vec<_> = (0..names.len()).filter(|&i| i != index).map(Some).collect();
    rebuild(pool, names, &slots, Vec::new())
}

/// `base` with the first free `_vN` suffix (N from 2), replacing one it already has.
pub fn unique_texture_name(existing: &[String], base: &str) -> String {
    let stem = match base.rsplit_once("_v") {
        Some((head, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => base,
    };
    (2..).map(|n| format!("{stem}_v{n}")).find(|c| !existing.contains(c)).unwrap_or_else(|| base.to_string())
}

/// An id for a new texture or primitive descriptor that nothing else in the file has. The game's ids come from the
/// name in a way Forge can't reproduce; what matters is that samplers and descriptors agree within the file. This is
/// the name's 32-bit FNV-1a, moved up past any taken id (and 0).
pub fn unused_descriptor_id(taken: &[u64], name: &str) -> u64 {
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    let mut id = hash & 0xffff_ffff;
    while id == 0 || taken.contains(&id) {
        id = (id + 1) & 0xffff_ffff;
    }
    id
}
