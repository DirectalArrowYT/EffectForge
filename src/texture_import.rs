//! Bring your own PNG: swap an image into an effect's BNTX texture pool, or pull one out.
//!
//! An eff's textures live in one multi-texture BNTX. `effect_library` can slice that pool
//! into single-texture BNTX exports and rebuild it from them, but the pixel payload stays
//! opaque to it — it copies bytes and never decodes. ScanMountGoat's `bntx` supplies the
//! other half: it parses a single-texture BNTX into an `image_dds` surface (undoing the
//! Tegra X1 block-linear swizzle) and builds one back from a surface.
//!
//! So a replacement is: slice every texture out of the pool, re-encode the one the user
//! picked from their PNG, and rebuild the pool from that set. Rebuilding from the ORIGINAL
//! pool as the base (rather than merging the exports into a fresh container) is deliberate —
//! it keeps the pool's own alignment, container name and string-table order, so a file with
//! one imported texture still differs from the game's only in the pixels that changed.
//!
//! Not every texture can make the trip. `bntx` models the surface format as a closed enum,
//! and the game corpus contains 102 textures (of 11915) in a format it does not list; those
//! fail to parse. They report an error rather than silently importing as something else.

use std::io::Cursor;

use anyhow::{anyhow, bail, Context, Result};
use binrw::BinRead;
use bntx::{Bntx, SurfaceFormat};
use image_dds::{ImageFormat, Mipmaps, Quality, SurfaceRgba8};

/// What one pool texture is, for the editor's picker and for round-trip decisions.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureDesc {
    pub width: u32,
    pub height: u32,
    pub mipmaps: u32,
    /// Format name as `bntx` calls it, e.g. `BC7Srgb` — shown verbatim in the UI.
    pub format: String,
    /// Whether this texture's format can be decoded to a PNG and encoded back.
    pub convertible: bool,
}

/// Read the pool's texture `index` back out as its own single-texture BNTX.
///
/// `effect_library` builds these for its own merge paths; reusing that builder means an
/// imported texture and an untouched one travel through exactly the same container code.
fn slice_one(pool: &[u8], index: usize, name: &str) -> Result<Vec<u8>> {
    effect_library::bntx::build_single_texture_bntx_public(pool, index, name)
        .with_context(|| format!("slicing texture '{name}' out of the pool"))
}

fn parse_single(bntx_bytes: &[u8], name: &str) -> Result<Bntx> {
    let mut cursor = Cursor::new(bntx_bytes);
    Bntx::read_le(&mut cursor).map_err(|e| anyhow!("bntx could not parse '{name}': {e}"))
}

fn le_u16(data: &[u8], at: usize) -> Result<u16> {
    data.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| anyhow!("BNTX truncated at {at:#x}"))
}

fn le_u32(data: &[u8], at: usize) -> Result<u32> {
    data.get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| anyhow!("BNTX truncated at {at:#x}"))
}

fn le_u64(data: &[u8], at: usize) -> Result<u64> {
    data.get(at..at + 8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| anyhow!("BNTX truncated at {at:#x}"))
}

/// Offset of `comp_sel` within a BRTI block — the channel swizzle.
///
/// One byte per output channel (R, G, B, A), each naming the SOURCE it reads from: 0 zero,
/// 1 one, 2 red, 3 green, 4 blue, 5 alpha.
///
/// This is how a single-channel texture reaches a shader. BC4 stores exactly one channel, and
/// the game's own textures broadcast it to all four — `0x02020202`, "every channel reads red" —
/// so a mask sampled as `texture.a` returns the shape. The identity swizzle `0x05040302` says
/// "alpha comes from alpha", and for BC4 there is no alpha plane, so the hardware returns 1.0
/// for every pixel: the effect draws as a fully opaque square no matter what the image contains.
///
/// So the swizzle is part of what a texture MEANS, not incidental header noise, and an import
/// has to carry the original's over. `bntx` writes the identity swizzle for everything it
/// builds, which is correct only for the four-channel formats.
const BRTI_COMP_SEL: usize = 0x58;

/// Offset of texture `index`'s BRTI block inside a pool.
fn brti_offset(pool: &[u8], index: usize) -> Result<usize> {
    let count = le_u32(pool, 0x24)? as usize;
    if index >= count {
        bail!("texture index {index} is outside a pool of {count}");
    }
    let table = le_u64(pool, 0x28)? as usize;
    let brti = le_u64(pool, table + index * 8)? as usize;
    if pool.get(brti..brti + 4) != Some(b"BRTI") {
        bail!("texture {index} does not start with a BRTI block");
    }
    Ok(brti)
}

/// Describe texture `index` of the pool.
///
/// Deliberately reads the BRTI header directly instead of going through `bntx`: `bntx`
/// models the surface format as a closed enum and refuses the whole container when it meets
/// one it does not list, which is exactly the case this needs to REPORT rather than fail on.
/// 102 of the corpus's 11915 textures are such a format.
pub fn describe(pool: &[u8], index: usize, _name: &str) -> Result<TextureDesc> {
    let brti = brti_offset(pool, index)?;
    let format_code = le_u32(pool, brti + 0x1C)?;
    // Round-trip the code through `bntx`'s own enum rather than keeping a second copy of the
    // table here: whatever it accepts is exactly what the encode/decode paths below accept.
    let format = SurfaceFormat::read_le(&mut Cursor::new(format_code.to_le_bytes())).ok();
    Ok(TextureDesc {
        width: le_u32(pool, brti + 0x24)?,
        height: le_u32(pool, brti + 0x28)?,
        mipmaps: le_u16(pool, brti + 0x16)? as u32,
        format: match format {
            Some(f) => format!("{f:?}"),
            None => format!("unknown ({format_code:#06x})"),
        },
        convertible: format
            .and_then(|f| ImageFormat::try_from(f).ok())
            .is_some_and(|f| !is_signed(f)),
    })
}

/// Does this format store SIGNED values?
///
/// Signed formats span −1..1, and every path here goes through 8-bit unsigned RGBA, which cannot
/// represent the negative half. Decoding one and re-encoding it does not round-trip: measured
/// against the corpus, `ef_mario_localcoin00_nor` (BC5Snorm) came back with a mean error of 51
/// levels per channel — a normal map turned into noise.
///
/// So they are reported as unconvertible, which is what they are, rather than importing as
/// something the game will render wrong. This is the same rule the module already applies to
/// formats `bntx` cannot model: refuse rather than silently substitute.
fn is_signed(format: ImageFormat) -> bool {
    matches!(
        format,
        ImageFormat::R8Snorm
            | ImageFormat::Rg8Snorm
            | ImageFormat::Rgba8Snorm
            | ImageFormat::BC4RSnorm
            | ImageFormat::BC5RgSnorm
            | ImageFormat::R16Snorm
            | ImageFormat::Rg16Snorm
            | ImageFormat::Rgba16Snorm
    )
}

/// Decode texture `index` of the pool to straight RGBA8 pixels.
///
/// `max_edge` caps the longest side — pass `None` for the real thing, or a small number for a
/// preview. Downscaling here rather than at the call site means a 1024² BC7 sheet is decoded
/// once and the caller only ever holds the pixels it will actually draw.
pub fn decode_rgba(
    pool: &[u8],
    index: usize,
    name: &str,
    max_edge: Option<u32>,
) -> Result<image::RgbaImage> {
    let desc = describe(pool, index, name)?;
    if !desc.convertible {
        bail!(
            "'{name}' is {} — Effect Forge cannot convert that format",
            desc.format
        );
    }
    let single = slice_one(pool, index, name)?;
    let bntx = parse_single(&single, name)?;
    let surface = bntx
        .to_surface()
        .map_err(|e| anyhow!("cannot read '{name}' as an image: {e}"))?;
    let rgba = surface
        .decode_rgba8()
        .map_err(|e| anyhow!("cannot decode '{name}' ({}): {e}", desc.format))?;
    let image = rgba
        .to_image(0)
        .map_err(|e| anyhow!("cannot build an image from '{name}': {e}"))?;
    Ok(fit(image, max_edge))
}

/// A pool texture as a shader samples it: every mip level as RGBA8 with the BRTI channel swizzle
/// ([`BRTI_COMP_SEL`]) already applied, and whether the format is sRGB (the sampler linearises it).
///
/// [`decode_rgba`] is the picture of a texture, for showing and editing; this is what the game's
/// shaders read, which is what drawing with them needs: a BC4 mask sampled as `.a` reads the
/// broadcast red, and the shaders pick their own mip levels.
pub struct SampledTexture {
    pub levels: Vec<image::RgbaImage>,
    pub srgb: bool,
}

pub fn decode_for_sampling(pool: &[u8], index: usize, name: &str) -> Result<SampledTexture> {
    let desc = describe(pool, index, name)?;
    if !desc.convertible {
        bail!("'{name}' is {} — Effect Forge cannot convert that format", desc.format);
    }
    let comp_sel = le_u32(pool, brti_offset(pool, index)? + BRTI_COMP_SEL)?.to_le_bytes();
    let bntx = parse_single(&slice_one(pool, index, name)?, name)?;
    let surface = bntx
        .to_surface()
        .map_err(|e| anyhow!("cannot read '{name}' as an image: {e}"))?;
    let rgba = surface
        .decode_rgba8()
        .map_err(|e| anyhow!("cannot decode '{name}' ({}): {e}", desc.format))?;
    let mut levels = Vec::new();
    for level in 0..rgba.mipmaps.max(1) {
        let mut image = rgba
            .to_image(level)
            .map_err(|e| anyhow!("cannot build mip {level} of '{name}': {e}"))?;
        if comp_sel != [2, 3, 4, 5] {
            for pixel in image.pixels_mut() {
                let src = pixel.0;
                pixel.0 = comp_sel.map(|sel| match sel {
                    0 => 0,
                    2..=5 => src[sel as usize - 2],
                    _ => 255,
                });
            }
        }
        levels.push(image);
    }
    Ok(SampledTexture { levels, srgb: desc.format.contains("Srgb") })
}


/// Cap the longest side, preserving aspect. `None` leaves the image alone.
fn fit(image: image::RgbaImage, max_edge: Option<u32>) -> image::RgbaImage {
    match max_edge {
        Some(edge) if image.width() > edge || image.height() > edge => {
            let scale = edge as f32 / image.width().max(image.height()) as f32;
            let (w, h) = (
                ((image.width() as f32 * scale).round() as u32).max(1),
                ((image.height() as f32 * scale).round() as u32).max(1),
            );
            image::imageops::thumbnail(&image, w, h)
        }
        _ => image,
    }
}

/// RGB channels within this much of each other still count as "one flat colour". BCn decodes a
/// pure-white block back to exactly 255, so this only has to absorb the odd rounding artefact.
const MATTE_RGB_TOLERANCE: u8 = 4;

/// How far brightness and alpha may drift apart and still be considered the same channel.
const MASK_TOLERANCE: u8 = 6;

/// Which form of a texture is being handed around.
///
/// Effect textures are not pictures — they are masks, and the game stores them in whichever
/// packing is cheapest, then relies on the channel swizzle to present them to the shader. Edited
/// in that stored packing they are close to unintelligible: a BC5 mask exports as a red-and-green
/// image whose GREEN channel is secretly the transparency, and a BC3 mask exports as a blank
/// white square whose only content is invisible.
///
/// So everything the user sees or paints goes through [`Form::Editable`], and the raw packing
/// stays available for anyone who needs byte-level control.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Form {
    /// Swizzle applied, and anything carrying a single channel of information flattened to an
    /// opaque black-and-white image where black means "nothing is drawn".
    #[default]
    Editable,
    /// The stored channels exactly as the file holds them.
    Raw,
}

/// What a texture actually carries, once the swizzle has been applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layout {
    /// One channel of information: brightness and alpha are the same thing. Every BC4 mask.
    Mask,
    /// A flat colour with the shape in alpha. Most BC3 masks — a white field plus a silhouette.
    Matte { color: [u8; 3] },
    /// Fully opaque; everything is in the colour channels. Normal maps and colour sheets.
    Opaque,
    /// Independent colour and alpha, both meaningful. Kept as straight RGBA.
    ColorAlpha,
}

/// Perceived brightness — the shape channel on the way back in.
fn luminance(px: &image::Rgba<u8>) -> u8 {
    ((px[0] as u32 * 299 + px[1] as u32 * 587 + px[2] as u32 * 114) / 1000).min(255) as u8
}

/// The four `comp_sel` bytes for texture `index`: the source each output channel reads from.
fn comp_sel(pool: &[u8], index: usize) -> Result<[u8; 4]> {
    Ok(le_u32(pool, brti_offset(pool, index)? + BRTI_COMP_SEL)?.to_le_bytes())
}

/// Stored channels → what the shader samples.
fn apply_swizzle(image: &image::RgbaImage, sel: [u8; 4]) -> image::RgbaImage {
    let mut out = image.clone();
    for px in out.pixels_mut() {
        let src = *px;
        for (channel, source) in sel.iter().enumerate() {
            px[channel] = match source {
                0 => 0,
                1 => 255,
                2..=5 => src[(source - 2) as usize],
                // Undocumented source: leave the channel as it was rather than invent a value.
                _ => src[channel],
            };
        }
    }
    out
}

/// What the shader samples → stored channels, the inverse of [`apply_swizzle`].
///
/// A stored channel no output reads back is unrecoverable, and gets a neutral value. That is
/// never a loss in practice: those are exactly the channels the format does not encode — BC4
/// keeps only red, BC5 only red and green.
fn unapply_swizzle(image: &image::RgbaImage, sel: [u8; 4]) -> image::RgbaImage {
    let read_by: [Option<usize>; 4] =
        [0, 1, 2, 3].map(|stored| (0..4).find(|&out| sel[out] as usize == stored + 2));
    let mut out = image.clone();
    for px in out.pixels_mut() {
        let sampled = *px;
        for stored in 0..4 {
            px[stored] = match read_by[stored] {
                Some(from) => sampled[from],
                None if stored == 3 => 255,
                None => 0,
            };
        }
    }
    out
}

/// Classify a SAMPLED image — the swizzle must already have been applied.
fn classify(image: &image::RgbaImage) -> Layout {
    let mut lo = [255u8; 3];
    let mut hi = [0u8; 3];
    let mut opaque = true;
    let mut mask_like = true;
    for px in image.pixels() {
        for c in 0..3 {
            lo[c] = lo[c].min(px[c]);
            hi[c] = hi[c].max(px[c]);
        }
        opaque &= px[3] == 255;
        mask_like &= luminance(px).abs_diff(px[3]) <= MASK_TOLERANCE;
    }
    if opaque {
        return Layout::Opaque;
    }
    if mask_like {
        return Layout::Mask;
    }
    if (0..3).all(|c| hi[c].saturating_sub(lo[c]) <= MATTE_RGB_TOLERANCE) {
        return Layout::Matte {
            color: [0, 1, 2].map(|c| ((lo[c] as u16 + hi[c] as u16) / 2) as u8),
        };
    }
    Layout::ColorAlpha
}


/// The shape a painted mask is asking for: whichever of brightness or alpha says "less".
///
/// [`to_editable`] writes the same value into both, so an untouched export gives them back
/// identical and this is exact. Once edited they can disagree, and taking the minimum is what
/// lets EITHER edit mean what it looks like — painting black erases, and so does erasing. It
/// also absorbs the two ways a paint program mangles this kind of file on save: flattening
/// alpha to opaque (brightness still carries the shape) and compositing colour onto white
/// (alpha still carries it).
fn shape_of(px: &image::Rgba<u8>) -> u8 {
    luminance(px).min(px[3])
}

/// The image the user painted → sampled pixels, the inverse of [`to_editable`].
fn from_editable(image: &image::RgbaImage, layout: Layout) -> image::RgbaImage {
    let mut out = image.clone();
    match layout {
        Layout::Mask => {
            for px in out.pixels_mut() {
                let v = shape_of(px);
                *px = image::Rgba([v, v, v, v]);
            }
        }
        Layout::Matte { color } => {
            for px in out.pixels_mut() {
                *px = image::Rgba([color[0], color[1], color[2], shape_of(px)]);
            }
        }
        Layout::Opaque => {
            for px in out.pixels_mut() {
                px[3] = 255;
            }
        }
        Layout::ColorAlpha => {}
    }
    out
}

/// The layout of texture `index`, and the swizzle needed to get to and from it.
fn layout_of(pool: &[u8], index: usize, name: &str) -> Result<([u8; 4], Layout)> {
    let sel = comp_sel(pool, index)?;
    let stored = decode_rgba(pool, index, name, None)?;
    Ok((sel, classify(&apply_swizzle(&stored, sel))))
}


/// The image the PANEL draws for `form`.
///
/// `Editable` is what the shader samples: the swizzle applied, alpha intact. A mask therefore
/// previews as a shape over the checkerboard — you can see the transparency, which is the whole
/// point of looking at it. Flattening it to the paintable matte here would hide exactly the
/// property the preview exists to show.
fn preview_for_form(stored: &image::RgbaImage, sel: [u8; 4], form: Form) -> image::RgbaImage {
    match form {
        Form::Raw => stored.clone(),
        Form::Editable => apply_swizzle(stored, sel),
    }
}


/// Decode texture `index` as the panel shows it.
pub fn decode_preview(
    pool: &[u8],
    index: usize,
    name: &str,
    form: Form,
    max_edge: Option<u32>,
) -> Result<image::RgbaImage> {
    let stored = decode_rgba(pool, index, name, None)?;
    let sel = comp_sel(pool, index).unwrap_or([2, 3, 4, 5]);
    Ok(fit(preview_for_form(&stored, sel, form), max_edge))
}




/// What an import actually did, so the UI can say it rather than imply it.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportReport {
    pub width: u32,
    pub height: u32,
    /// The format the new pixels were encoded to.
    pub format: String,
    /// Set when the original format could not be re-encoded and a substitute was used.
    pub format_substituted_from: Option<String>,
    pub mipmaps: u32,
    /// How the incoming image was read.
    pub form: Form,
    /// What the texture it replaced turned out to carry — `None` when read raw.
    pub layout: Option<Layout>,
}

/// Turn an imported PNG into the STORED channels to encode.
fn stored_from_png(
    pool: &[u8],
    index: usize,
    name: &str,
    image: &image::RgbaImage,
    form: Form,
) -> (image::RgbaImage, Option<Layout>) {
    if form == Form::Raw {
        return (image.clone(), None);
    }
    // A texture whose layout cannot be read is passed through rather than mangled by a guess.
    let Ok((sel, layout)) = layout_of(pool, index, name) else {
        return (image.clone(), None);
    };
    (
        unapply_swizzle(&from_editable(image, layout), sel),
        Some(layout),
    )
}


/// Encode a PNG into a single-texture BNTX matching what texture `template` already is.
///
/// The format, the channel swizzle and the DIMENSIONS all come from the template. Dimensions are
/// a hard requirement rather than a courtesy: emitters carry UV rects and scroll rates sized for
/// the texture they were authored against, so a different-sized image does not "just scale" — it
/// resamples the effect. The template is the texture the new pixels are standing in for, whether
/// that is the one being replaced or the one a new entry is taking over from.
fn encode_like(
    pool: &[u8],
    names: &[String],
    template: usize,
    output_name: &str,
    png: &[u8],
    form: Form,
) -> Result<(Vec<u8>, ImportReport)> {
    let template_name = names.get(template).ok_or_else(|| {
        anyhow!(
            "texture index {template} is outside a pool of {}",
            names.len()
        )
    })?;

    // Read the target's shape before touching the PNG, so an unreadable one fails before the
    // (slow) BCn encode.
    let desc = describe(pool, template, template_name)?;
    let original_format = SurfaceFormat::read_le(&mut Cursor::new(
        le_u32(pool, brti_offset(pool, template)? + 0x1C)?.to_le_bytes(),
    ))
    .ok();
    let (target, substituted_from) = match original_format.map(ImageFormat::try_from) {
        // BC6h encodes from float data; an 8-bit PNG has nothing to give it.
        Some(Ok(f)) if !matches!(f, ImageFormat::BC6hRgbUfloat | ImageFormat::BC6hRgbSfloat) => {
            (f, None)
        }
        _ => (ImageFormat::BC7RgbaUnormSrgb, Some(desc.format.clone())),
    };
    // Only meaningful while the format is unchanged: a substituted BC7 really does have four
    // channels, so the identity swizzle `bntx` writes is the right one for it.
    let keep_comp_sel = substituted_from
        .is_none()
        .then(|| le_u32(pool, brti_offset(pool, template)? + BRTI_COMP_SEL))
        .transpose()?;

    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .context("reading the PNG")?
        .to_rgba8();
    if image.width() == 0 || image.height() == 0 {
        bail!("the PNG is empty");
    }
    if (image.width(), image.height()) != (desc.width, desc.height) {
        bail!(
            "that image is {}×{}, but '{template_name}' is {}×{} — effect textures have to keep \
             their size, because emitters carry UV rects authored against it. Resize the image \
             and try again.",
            image.width(),
            image.height(),
            desc.width,
            desc.height
        );
    }
    // Back to the packing the file uses. Skipping this is what made an edited mask encode as
    // alpha=255 everywhere, so the game drew a solid square instead of the shape.
    let (image, layout) = stored_from_png(pool, template, template_name, &image, form);

    let encoded = SurfaceRgba8::from_image(&image)
        .encode(target, Quality::Normal, Mipmaps::GeneratedAutomatic)
        .map_err(|e| anyhow!("encoding the PNG as {target:?}: {e}"))?;
    let report = ImportReport {
        width: encoded.width,
        height: encoded.height,
        format: format!("{target:?}"),
        format_substituted_from: substituted_from,
        mipmaps: encoded.mipmaps,
        form,
        layout,
    };

    let mut built = Bntx::from_surface(encoded, output_name)
        .map_err(|e| anyhow!("building a BNTX for '{output_name}': {e}"))?;
    if let (Some(comp_sel), Some(brti)) = (keep_comp_sel, built.nx_header.brtis.first_mut()) {
        brti.brti.comp_sel = comp_sel;
    }
    let mut bytes = Cursor::new(Vec::new());
    built
        .write(&mut bytes)
        .with_context(|| format!("writing the BNTX for '{output_name}'"))?;
    Ok((bytes.into_inner(), report))
}

/// Rebuild the pool from one single-texture BNTX per surviving texture.
///
/// `slots` is the pool order to produce: `Some(i)` passes existing texture `i` through untouched,
/// `None` takes the next of `added`. Dropping an index deletes it. The eff serializer re-sorts
/// descriptors by name and reorders the archive to match on save, so this order only has to be
/// self-consistent, not sorted.
fn rebuild(
    pool: &[u8],
    names: &[String],
    slots: &[Option<usize>],
    added: Vec<Vec<u8>>,
) -> Result<Vec<u8>> {
    let mut added = added.into_iter();
    let mut exports = Vec::with_capacity(slots.len());
    for slot in slots {
        match slot {
            Some(i) => exports.push(slice_one(pool, *i, &names[*i])?),
            None => exports.push(
                added
                    .next()
                    .ok_or_else(|| anyhow!("internal: fewer new textures than pool slots"))?,
            ),
        }
    }
    effect_library::bntx::rebuild_from_base_and_exports(pool, &exports)
        .context("rebuilding the texture pool")
}

/// Encode `png` over texture `index`, returning the rebuilt pool.
///
/// `names` must be the pool's texture names in pool order — the same order the eff's texture
/// descriptors are in, which is what the caller already holds.
///
/// The new pixels are encoded to the format the texture already used, so an imported normal
/// map stays BC5 and a colour sheet stays BC3/BC7. When the original format cannot be
/// encoded from RGBA (BC6h's float targets, or a format `bntx` does not model), BC7Srgb is
/// substituted and [`ImportReport::format_substituted_from`] records what was replaced.
///
/// The original's channel swizzle is carried over too — see [`BRTI_COMP_SEL`]. Without that a
/// re-encoded single-channel texture is structurally perfect and renders as a solid square.
pub fn replace_with_png(
    pool: &[u8],
    names: &[String],
    index: usize,
    png: &[u8],
    form: Form,
) -> Result<(Vec<u8>, ImportReport)> {
    let name = names
        .get(index)
        .ok_or_else(|| anyhow!("texture index {index} is outside a pool of {}", names.len()))?
        .clone();
    let (replacement, report) = encode_like(pool, names, index, &name, png, form)?;
    // Same slots, same names — only the one texture's payload differs.
    let slots: Vec<Option<usize>> = (0..names.len())
        .map(|i| (i != index).then_some(i))
        .collect();
    Ok((rebuild(pool, names, &slots, vec![replacement])?, report))
}

/// Append a copy of texture `src` under `new_name`, returning the rebuilt pool.
///
/// The point is ISOLATION. A pool texture is shared by every emitter that samples it — and
/// `ef_cmn_*` names are shared by dozens of effects within one eff — so editing one to change a
/// single effect changes all of them. A private copy is the only way to alter one effect's
/// texture and leave the rest alone.
///
/// The copy is byte-identical: it is sliced straight out of the pool and only renamed, so no
/// re-encode and no generation loss.
pub fn duplicate_texture(
    pool: &[u8],
    names: &[String],
    src: usize,
    new_name: &str,
) -> Result<Vec<u8>> {
    if src >= names.len() {
        bail!("texture index {src} is outside a pool of {}", names.len());
    }
    if names.iter().any(|n| n == new_name) {
        bail!("this eff already holds a texture called '{new_name}'");
    }
    let copy = slice_one(pool, src, new_name)?;
    let mut slots: Vec<Option<usize>> = (0..names.len()).map(Some).collect();
    slots.push(None);
    rebuild(pool, names, &slots, vec![copy])
}

/// Append a new texture built from `png`, shaped like texture `template`.
pub fn add_texture_from_png(
    pool: &[u8],
    names: &[String],
    template: usize,
    new_name: &str,
    png: &[u8],
    form: Form,
) -> Result<(Vec<u8>, ImportReport)> {
    if names.iter().any(|n| n == new_name) {
        bail!("this eff already holds a texture called '{new_name}'");
    }
    let (built, report) = encode_like(pool, names, template, new_name, png, form)?;
    let mut slots: Vec<Option<usize>> = (0..names.len()).map(Some).collect();
    slots.push(None);
    Ok((rebuild(pool, names, &slots, vec![built])?, report))
}

/// Drop texture `index` from the pool, returning the rebuilt pool.
///
/// Says nothing about whether anything still samples it — the caller owns that check, because
/// only the caller can see the emitters.
pub fn remove_texture(pool: &[u8], names: &[String], index: usize) -> Result<Vec<u8>> {
    if index >= names.len() {
        bail!("texture index {index} is outside a pool of {}", names.len());
    }
    if names.len() == 1 {
        bail!(
            "'{}' is the only texture in this eff — removing it would leave the pool empty, \
               which is not a shape this writer has been calibrated against",
            names[index]
        );
    }
    let slots: Vec<Option<usize>> = (0..names.len()).filter(|i| *i != index).map(Some).collect();
    rebuild(pool, names, &slots, Vec::new())
}

/// A pool-unique name for a new texture derived from `base`.
///
/// Kept short: these end up in the BNTX string table and the descriptor sort, and the game's own
/// names run to about 24 characters.
pub fn unique_texture_name(existing: &[String], base: &str) -> String {
    // Don't stack suffixes on a name that already has one.
    let stem = match base.rsplit_once("_v") {
        Some((head, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => head,
        _ => base,
    };
    (2..)
        .map(|n| format!("{stem}_v{n}"))
        .find(|candidate| !existing.iter().any(|n| n == candidate))
        .unwrap_or_else(|| base.to_string())
}

/// A descriptor id no other texture in this eff uses.
///
/// The game's own ids are a 1:1 function of the texture name that nothing in this toolchain can
/// compute — no CRC variant matches and it is not affine over GF(2), so it cannot be reproduced.
/// What the format needs from an id is that emitters and descriptors agree WITHIN the file: the
/// pruner, the transplant merge and `apply_authored` all resolve textures by matching a sampler's
/// `texture_id` to a descriptor `id`, and the eff is self-contained.
///
/// So a new texture gets a derived-but-arbitrary id, probed until it is unique here. If it turns
/// out the runtime recomputes ids from names, a duplicated texture will fail to bind and render
/// as a solid square — the one thing to watch for on the first in-game test.
pub fn unused_descriptor_id(taken: &[u64], name: &str) -> u64 {
    // FNV-1a, so the same name yields the same id run to run and diffs stay readable.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in name.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    let mut candidate = hash & 0xFFFF_FFFF;
    while candidate == 0 || taken.contains(&candidate) {
        candidate = (candidate + 1) & 0xFFFF_FFFF;
    }
    candidate
}
