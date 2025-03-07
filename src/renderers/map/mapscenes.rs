use std::collections::BTreeMap;

use image::{GenericImage, GenericImageView, RgbaImage};
use itertools::iproduct;

#[cfg(any(feature = "rs3", feature = "2009_1_shim"))]
use crate::definitions::mapscenes::MapScene;
use crate::{
    definitions::{location_configs::LocationConfig, mapsquares::GroupMapSquare, sprites::Sprite},
    renderers::map::CONFIG,
    utils::rangeclamp::RangeClamp,
};

/// Applies [`MapScene`]s to the base image.
pub fn put(
    plane: usize,
    img: &mut RgbaImage,
    squares: &GroupMapSquare,
    location_config: &BTreeMap<u32, LocationConfig>,
    #[cfg(any(feature = "rs3", feature = "2009_1_shim"))] mapscenes: &BTreeMap<u32, MapScene>,
    sprites: &BTreeMap<(u32, u32), Sprite>,
) {
    squares
        .all_locations_iter()
        .filter_map(|loc| {
            if loc.plane.matches(&(plane as u8)) {
                location_config
                    .get(&(loc.id))
                    .unwrap_or_else(|| {
                        panic!(
                            "fatal error: maps and location_configs are logically inconsistent, tried looking up {} of mapsquare {}_{}",
                            loc.id,
                            squares.core_i(),
                            squares.core_j()
                        );
                    })
                    .mapscene
                    .and_then(|mapscene_id| {
                        #[cfg(any(feature = "rs3", feature = "2009_1_shim"))]
                        {
                            mapscenes[&(mapscene_id as u32)]
                                .sprite_id
                                // sprites is constructed with ids from
                                // mapscenes so it should always be in the map.
                                .map(|sprite_id| (loc, &sprites[&(sprite_id, 0)]))
                        }

                        #[cfg(all(feature = "osrs", not(feature = "2009_1_shim")))]
                        {
                            // 317 is the sprite named "mapscene", whose frames form all the mapscenes.
                            // 22 is missing and indicates the empty mapscene, which is why this does not index
                            sprites.get(&(317, mapscene_id as u32)).map(|s| (loc, s))
                        }

                        #[cfg(feature = "legacy")]
                        {
                            sprites.get(&(317, mapscene_id as u32)).map(|s| (loc, s))
                        }
                    })
            } else {
                None
            }
        })
        .for_each(|(loc, sprite)| {
            let offset_a = CONFIG.tile_size as i32 * ((loc.i as i32 - squares.core_i() as i32) * 64 + loc.x as i32);
            let offset_b = CONFIG.tile_size as i32 * (63 - (loc.j as i32 - squares.core_j() as i32) * 64 - loc.y as i32);

            let dim_a = sprite.width() as i32;
            let dim_b = sprite.height() as i32;

            // There is an offset here that's not present in osrs
            let vertical_offset = if cfg!(feature = "rs3") { dim_b / 2 } else { 0 };

            let range_a = (offset_a..(offset_a + dim_a)).clamp(0, img.width() as i32);
            let range_b = ((offset_b - vertical_offset)..(offset_b + dim_b - vertical_offset)).clamp(0, img.height() as i32);

            for (a, b) in iproduct!(range_a, range_b) {
                let sprite_a = (a - offset_a) as u32;
                let sprite_b = (b - (offset_b - vertical_offset)) as u32;

                let sprite_pixel = unsafe {
                    debug_assert!(sprite_a < sprite.width() && sprite_b < sprite.height(), "Index out of range.");
                    sprite.unsafe_get_pixel(sprite_a, sprite_b)
                };
                if sprite_pixel[3] != 0 {
                    unsafe {
                        debug_assert!((a as u32) < img.width() && (b as u32) < img.height(), "Index out of range.");
                        img.unsafe_put_pixel(a as u32, b as u32, sprite_pixel)
                    };
                }
            }
        });
}
