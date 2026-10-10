// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! The pre-rendered textures and bitmap fonts of a compilation unit, in the form the
//! software renderer takes them.
//!
//! The renderer only takes `'static` data, which generated code keeps in statics.
//! Here each distinct texture or font is leaked once, so that reloading a design with
//! the same resources doesn't grow the heap.

use crate::prelude::*;
use core::cell::RefCell;
use core::hash::Hasher;
use i_slint_compiler::embedded_resources as unit;
use i_slint_core::graphics::{
    BitmapFont, BitmapGlyph, BitmapGlyphs, StaticTexture, StaticTextures, TexturePixelFormat,
};
use i_slint_core::slice::Slice;

i_slint_core::thread_local! {
    static TEXTURES: RefCell<Vec<(u64, &'static StaticTextures)>> = RefCell::default();
    static FONTS: RefCell<Vec<(u64, &'static BitmapFont)>> = RefCell::default();
}

/// The texture as the renderer takes it, leaked the first time its content is seen.
pub fn static_textures(texture: &unit::Texture) -> &'static StaticTextures {
    let hash = texture_hash(texture);
    TEXTURES.with(|textures| {
        let mut textures = textures.borrow_mut();
        if let Some((_, known)) =
            textures.iter().find(|(h, known)| *h == hash && same_texture(texture, known))
        {
            return *known;
        }
        let leaked = leak_texture(texture);
        textures.push((hash, leaked));
        leaked
    })
}

/// The font as the renderer takes it, leaked the first time its content is seen.
pub fn bitmap_font(font: &unit::BitmapFont) -> &'static BitmapFont {
    let hash = font_hash(font);
    FONTS.with(|fonts| {
        let mut fonts = fonts.borrow_mut();
        if let Some((_, known)) =
            fonts.iter().find(|(h, known)| *h == hash && same_font(font, known))
        {
            return *known;
        }
        let leaked = leak_font(font);
        fonts.push((hash, leaked));
        leaked
    })
}

fn leak<T>(values: Vec<T>) -> &'static [T] {
    Box::leak(values.into_boxed_slice())
}

/// `data`, in memory aligned for any pixel format the renderer reads from it.
fn leak_aligned(data: &[u8]) -> &'static [u8] {
    let words: &'static mut [u32] =
        Box::leak(vec![0u32; data.len().div_ceil(4)].into_boxed_slice());
    // SAFETY: `words` is a leaked allocation of at least `data.len()` bytes, and u8 has no
    // alignment requirement.
    let bytes =
        unsafe { core::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), data.len()) };
    bytes.copy_from_slice(data);
    bytes
}

fn pixel_format(format: unit::PixelFormat) -> TexturePixelFormat {
    match format {
        unit::PixelFormat::Rgb => TexturePixelFormat::Rgb,
        unit::PixelFormat::Rgba => TexturePixelFormat::Rgba,
        unit::PixelFormat::RgbaPremultiplied => TexturePixelFormat::RgbaPremultiplied,
        unit::PixelFormat::AlphaMap(_) => TexturePixelFormat::AlphaMap,
    }
}

fn leak_texture(texture: &unit::Texture) -> &'static StaticTextures {
    let color = match texture.format {
        unit::PixelFormat::AlphaMap([r, g, b]) => i_slint_core::Color::from_rgb_u8(r, g, b),
        _ => i_slint_core::Color::from_argb_encoded(0),
    };
    let rect = texture.rect;
    let textures = leak(vec![StaticTexture {
        rect: i_slint_core::graphics::euclid::rect(
            rect.x(),
            rect.y(),
            rect.width() as _,
            rect.height() as _,
        ),
        format: pixel_format(texture.format),
        color,
        index: 0,
    }]);
    Box::leak(Box::new(StaticTextures {
        size: i_slint_core::graphics::IntSize::new(
            texture.total_size.width,
            texture.total_size.height,
        ),
        original_size: i_slint_core::graphics::IntSize::new(
            texture.original_size.width,
            texture.original_size.height,
        ),
        data: Slice::from_slice(leak_aligned(&texture.data)),
        textures: Slice::from_slice(textures),
    }))
}

fn leak_font(font: &unit::BitmapFont) -> &'static BitmapFont {
    let character_map = leak(font.character_map.clone());
    // One allocation for all the glyph bitmaps of the font, not one per glyph.
    let all_data = leak(
        font.glyphs
            .iter()
            .flat_map(|glyphs| glyphs.glyph_data.iter())
            .flat_map(|glyph| glyph.data.iter().copied())
            .collect(),
    );
    let mut offset = 0;
    let glyphs = leak(
        font.glyphs
            .iter()
            .map(|glyphs| BitmapGlyphs {
                pixel_size: glyphs.pixel_size,
                glyph_data: Slice::from_slice(leak(
                    glyphs
                        .glyph_data
                        .iter()
                        .map(|glyph| {
                            let data = &all_data[offset..offset + glyph.data.len()];
                            offset += glyph.data.len();
                            BitmapGlyph {
                                x: glyph.x,
                                y: glyph.y,
                                width: glyph.width,
                                height: glyph.height,
                                x_advance: glyph.x_advance,
                                data: Slice::from_slice(data),
                            }
                        })
                        .collect(),
                )),
            })
            .collect(),
    );
    Box::leak(Box::new(BitmapFont {
        family_name: Slice::from_slice(leak(font.family_name.as_bytes().to_vec())),
        character_map: Slice::from_slice(character_map),
        units_per_em: font.units_per_em,
        ascent: font.ascent,
        descent: font.descent,
        x_height: font.x_height,
        cap_height: font.cap_height,
        glyphs: Slice::from_slice(glyphs),
        weight: font.weight,
        italic: font.italic,
        sdf: font.sdf,
    }))
}

/// FNV-1a, which needs no std and no random seed.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

fn texture_hash(texture: &unit::Texture) -> u64 {
    let mut hasher = Fnv::default();
    hasher.write(&texture.data);
    hasher.write_u32(texture.total_size.width);
    hasher.write_u32(texture.total_size.height);
    hasher.finish()
}

fn font_hash(font: &unit::BitmapFont) -> u64 {
    let mut hasher = Fnv::default();
    hasher.write(font.family_name.as_bytes());
    for glyphs in &font.glyphs {
        hasher.write_i16(glyphs.pixel_size);
        for glyph in &glyphs.glyph_data {
            hasher.write(&glyph.data);
        }
    }
    for code_point in &font.character_map {
        hasher.write_u32((*code_point).into());
    }
    hasher.finish()
}

fn same_texture(texture: &unit::Texture, known: &StaticTextures) -> bool {
    let [known_texture] = known.textures.as_slice() else { return false };
    let rect = texture.rect;
    known.data.as_slice() == texture.data.as_slice()
        && known.size.width == texture.total_size.width
        && known.size.height == texture.total_size.height
        && known.original_size.width == texture.original_size.width
        && known.original_size.height == texture.original_size.height
        && known_texture.format == pixel_format(texture.format)
        && known_texture.rect
            == i_slint_core::graphics::euclid::rect(
                rect.x(),
                rect.y(),
                rect.width() as _,
                rect.height() as _,
            )
        && match texture.format {
            unit::PixelFormat::AlphaMap([r, g, b]) => {
                known_texture.color == i_slint_core::Color::from_rgb_u8(r, g, b)
            }
            _ => true,
        }
}

fn same_font(font: &unit::BitmapFont, known: &BitmapFont) -> bool {
    known.family_name.as_slice() == font.family_name.as_bytes()
        && known.units_per_em == font.units_per_em
        && known.ascent == font.ascent
        && known.descent == font.descent
        && known.x_height == font.x_height
        && known.cap_height == font.cap_height
        && known.weight == font.weight
        && known.italic == font.italic
        && known.sdf == font.sdf
        && known.character_map.as_slice() == font.character_map.as_slice()
        && known.glyphs.len() == font.glyphs.len()
        && known.glyphs.iter().zip(&font.glyphs).all(|(k, g)| {
            k.pixel_size == g.pixel_size
                && k.glyph_data.len() == g.glyph_data.len()
                && k.glyph_data.iter().zip(&g.glyph_data).all(|(k, g)| {
                    k.x == g.x
                        && k.y == g.y
                        && k.width == g.width
                        && k.height == g.height
                        && k.x_advance == g.x_advance
                        && k.data.as_slice() == g.data.as_slice()
                })
        })
}

#[cfg(all(test, feature = "internal-serde", feature = "internal-embed-textures"))]
#[test]
fn the_same_resources_are_leaked_once() {
    let units: Vec<_> = (0..2)
        .map(|_| {
            let mut compiler = crate::Compiler::default();
            compiler.set_embed_resources(crate::EmbedResourcesKind::EmbedTextures);
            let result = spin_on::spin_on(compiler.build_from_source(
                "export component Live inherits Window { Text { text: \"Version 1\"; font-size: 32px; } }".into(),
                Default::default(),
            ));
            assert!(!result.has_errors(), "{:?}", result.diagnostics().collect::<Vec<_>>());
            let bytes = result.into_send().to_bytes(i_slint_core::InternalToken).unwrap();
            i_slint_compiler::llr::serialize::from_bytes(&bytes).unwrap()
        })
        .collect();
    let fonts = |unit: &i_slint_compiler::llr::CompilationUnit| -> Vec<*const BitmapFont> {
        unit.embedded_resources
            .iter()
            .filter_map(|resource| match resource {
                unit::EmbeddedResource::BitmapFont(font) => Some(bitmap_font(font) as *const _),
                _ => None,
            })
            .collect()
    };
    let first = fonts(&units[0]);
    assert!(!first.is_empty());
    assert_eq!(first, fonts(&units[1]));
}
