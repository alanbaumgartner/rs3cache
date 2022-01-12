//! Data comprising the game surface.
//!
//! The game map is divided in (potentially) 128 by 256 [`MapSquare`]s,
//! though this seems to be limited to 100 by 200.
//! These coordinates are referred to as `i` and `j`.
//!
//! Each [`MapSquare`] itself is comprised of 64 by 64 [`Tile`]s.
//! These coordinates are referred to as `x` and `y`.
//! They have four elevations, referred to as `p` or `plane`.

#[cfg_attr(feature = "rs3", path = "mapsquares/rs3.rs")]
#[cfg_attr(feature = "osrs", path = "mapsquares/osrs.rs")]
#[cfg_attr(feature = "legacy", path = "mapsquares/legacy.rs")]
mod iterator;

use std::{
    collections::{hash_map, BTreeMap, HashMap},
    fs::{self, File},
    io::Write,
    iter::Zip,
    ops::Range,
};

use fstrings::{f, format_args_f};
use itertools::{iproduct, Product};
use ndarray::{iter::LanesIter, s, Axis, Dim};
use path_macro::path;
use rayon::iter::{ParallelBridge, ParallelIterator};

pub use self::iterator::*;
#[cfg(feature = "rs3")]
use crate::cache::arc::Archive;
#[cfg(feature = "osrs")]
use crate::cache::xtea::Xtea;
use crate::{
    cache::{
        error::{CacheError, CacheResult},
        index::{CacheIndex, Initial},
        indextype::{IndexType, MapFileType},
    },
    definitions::{
        locations::Location,
        tiles::{Tile, TileArray},
    },
    utils::rangeclamp::RangeClamp,
};

/// Represents a section of the game map
#[derive(Debug)]
pub struct MapSquare {
    /// The horizontal [`MapSquare`] coordinate.
    ///
    /// It can have any value in the range `0..=100`.
    i: u8,

    /// The vertical [`MapSquare`] coordinate.
    ///
    /// It can have any value in the range `0..=200`.
    j: u8,

    /// Data on the tiles it contains.
    tiles: CacheResult<TileArray>,

    /// All locations in this [`MapSquare`].
    ///
    /// Locations can overlap on surrounding mapsquares.
    locations: CacheResult<Vec<Location>>,

    /// All water locations in this [`MapSquare`].
    ///
    /// Locations can overlap on surrounding mapsquares.
    #[cfg(feature = "rs3")]
    water_locations: CacheResult<Vec<Location>>,
}

/// Iterator over a columns of planes with their x, y coordinates
pub type ColumnIter<'c> = Zip<LanesIter<'c, Tile, Dim<[usize; 2]>>, Product<Range<u32>, Range<u32>>>;

impl MapSquare {
    /// The horizontal [`MapSquare`] coordinate.
    ///
    /// It can have any value in the range `0..=100`.
    pub fn i(&self) -> u8 {
        self.i
    }

    /// The vertical [`MapSquare`] coordinate.
    ///
    /// It can have any value in the range `0..=200`.
    pub fn j(&self) -> u8 {
        self.j
    }

    #[cfg(all(test, feature = "rs3"))]
    pub fn new(i: u8, j: u8, config: &crate::cli::Config) -> CacheResult<MapSquare> {
        assert!(i < 0x7F, "Index out of range.");
        let archive_id = (i as u32) | (j as u32) << 7;
        let archive = CacheIndex::new(IndexType::MAPSV2, &config.input)?.archive(archive_id)?;
        Ok(Self::from_archive(archive))
    }

    #[cfg(feature = "osrs")]
    fn new(index: &CacheIndex<Initial>, xtea: Option<Xtea>, land: u32, tiles: u32, env: Option<u32>, i: u8, j: u8) -> CacheResult<MapSquare> {
        let land = index.archive_with_xtea(land, xtea).and_then(|arch| arch.file(&0));
        let tiles = index.archive(tiles)?.file(&0)?;
        let _env = env.map(|k| index.archive(k));

        let tiles = Tile::dump(tiles);
        let locations = if let Ok(ok_land) = land {
            Ok(Location::dump(i, j, &tiles, ok_land))
        } else {
            Err(CacheError::ArchiveNotFoundError(5, 0))
        };

        Ok(MapSquare {
            i,
            j,
            tiles: Ok(tiles),
            locations,
        })
    }

    #[cfg(feature = "legacy")]
    fn new(index: &CacheIndex<Initial>, loc: u32, map: u32, i: u8, j: u8) -> CacheResult<MapSquare> {
        let land = index.archive(loc)?.file(&0)?;
        let tiles = index.archive(map)?.file(&0)?;

        let tiles = Tile::dump(tiles);
        let locations = Location::dump(i, j, &tiles, land);

        Ok(MapSquare {
            i,
            j,
            tiles: Ok(tiles),
            locations: Ok(locations),
        })
    }

    #[cfg(feature = "rs3")]
    pub(crate) fn from_archive(archive: Archive) -> MapSquare {
        let i = (archive.archive_id() & 0x7F) as u8;
        let j = (archive.archive_id() >> 7) as u8;
        let tiles = archive.file(&MapFileType::TILES).map(Tile::dump);
        let locations = match tiles {
            Ok(ref t) => archive.file(&MapFileType::LOCATIONS).map(|file| Location::dump(i, j, t, file)),
            // can't generally clone or copy error
            Err(CacheError::FileNotFoundError(i, a, f)) => Err(CacheError::FileNotFoundError(i, a, f)),
            _ => unreachable!(),
        };
        let water_locations = archive
            .file(&MapFileType::WATER_LOCATIONS)
            .map(|file| Location::dump_water_locations(i, j, file));

        MapSquare {
            i,
            j,
            tiles,
            locations,
            water_locations,
        }
    }

    /// Iterator over a columns of planes with their x, y coordinates
    pub fn indexed_columns(&self) -> Result<ColumnIter, &CacheError> {
        self.get_tiles()
            .map(|tiles| tiles.lanes(Axis(0)).into_iter().zip(iproduct!(0..64u32, 0..64u32)))
    }

    /// Returns a view over the `tiles` field, if present
    pub fn get_tiles(&self) -> Result<&TileArray, &CacheError> {
        self.tiles.as_ref()
    }

    /// Returns a view over the `tiles` field, if present
    pub fn take_tiles(self) -> Result<TileArray, CacheError> {
        self.tiles
    }

    /// Returns a view over the `locations` field, if present.
    pub fn get_locations(&self) -> Result<&Vec<Location>, &CacheError> {
        self.locations.as_ref()
    }

    /// Take its locations, consuming `self`.
    pub fn take_locations(self) -> Result<Vec<Location>, CacheError> {
        self.locations
    }

    /// Returns a view over the `locations` field, if present.
    #[cfg(feature = "rs3")]
    pub fn get_water_locations(&self) -> Result<&Vec<Location>, &CacheError> {
        self.water_locations.as_ref()
    }

    /// Take its locations, consuming `self`.
    #[cfg(feature = "rs3")]
    pub fn take_water_locations(self) -> Result<Vec<Location>, CacheError> {
        self.water_locations
    }
}

pub struct MapSquares {
    index: CacheIndex<Initial>,
    #[cfg(feature = "osrs")]
    mapping: BTreeMap<(&'static str, u8, u8), u32>,
    #[cfg(feature = "legacy")]
    meta: BTreeMap<(u8, u8), rs3cache_core::index::MapsquareMeta>,
}

impl IntoIterator for MapSquares {
    type Item = CacheResult<MapSquare>;
    type IntoIter = MapSquareIterator;

    #[cfg(feature = "rs3")]
    fn into_iter(self) -> Self::IntoIter {
        let state = self
            .index
            .metadatas()
            .keys()
            .map(|id| ((id & 0x7F) as u8, (id >> 7) as u8))
            .collect::<Vec<_>>()
            .into_iter();
        MapSquareIterator { mapsquares: self, state }
    }

    #[cfg(feature = "osrs")]
    fn into_iter(self) -> Self::IntoIter {
        let state = self
            .mapping
            .keys()
            .filter_map(|(ty, i, j)| if *ty == "m" { Some((*i, *j)) } else { None })
            .collect::<Vec<_>>()
            .into_iter();
        MapSquareIterator { mapsquares: self, state }
    }

    #[cfg(feature = "legacy")]
    fn into_iter(self) -> Self::IntoIter {
        let state = self.meta.keys().copied().collect::<Vec<_>>().into_iter();
        MapSquareIterator { mapsquares: self, state }
    }
}

/// A group of adjacent [`MapSquare`]s.
///
/// Necessary for operations that need to care about surrounding mapsquares.
pub struct GroupMapSquare {
    core_i: u8,
    core_j: u8,
    mapsquares: HashMap<(u8, u8), MapSquare>,
}

impl GroupMapSquare {
    /// The horizontal coordinate of the central [`MapSquare`].
    ///
    /// It can have any value in the range `0..100`.
    #[inline(always)]
    pub fn core_i(&self) -> u8 {
        self.core_i
    }

    /// The vertical coordinate of the central [`MapSquare`].
    ///
    /// It can have any value in the range `0..200`.
    #[inline(always)]
    pub fn core_j(&self) -> u8 {
        self.core_j
    }

    /// Returns a reference to the central [`MapSquare`].
    pub fn core(&self) -> Option<&MapSquare> {
        self.mapsquares.get(&(self.core_i, self.core_j))
    }

    /// Iterates over all [`MapSquare`]s of `self` in arbitrary order.
    pub fn iter(&self) -> hash_map::Iter<'_, (u8, u8), MapSquare> {
        self.mapsquares.iter()
    }

    /// Returns a view over a specific [`MapSquare`]..
    pub fn get(&self, key: &(u8, u8)) -> Option<&MapSquare> {
        self.mapsquares.get(key)
    }

    /// Returns a view over all tiles within `interp` of the [`Tile`] at `plane, x, y`.
    pub fn tiles_iter(&self, plane: usize, x: usize, y: usize, interp: isize) -> Box<dyn Iterator<Item = &Tile> + '_> {
        let low_x = x as isize - interp;
        let upper_x = x as isize + interp + 1;
        let low_y = y as isize - interp;
        let upper_y = y as isize + interp + 1;

        Box::new(
            self.iter()
                .filter_map(move |((i, j), sq)| {
                    sq.get_tiles().ok().map(|tiles| {
                        let di = (*i as isize) - (self.core_i as isize);
                        let dj = (*j as isize) - (self.core_j as isize);
                        ((di, dj), tiles)
                    })
                })
                .flat_map(move |((di, dj), tiles)| {
                    tiles
                        .slice(s![
                            plane,
                            ((low_x - 64 * di)..(upper_x - 64 * di)).clamp(0, 64),
                            ((low_y - 64 * dj)..(upper_y - 64 * dj)).clamp(0, 64)
                        ])
                        .into_iter()
                }),
        )
    }

    /// Returns a view over all locations in all [`MapSquare`]s of `self` in arbitrary order.
    pub fn all_locations_iter(&self) -> Box<dyn Iterator<Item = &Location> + '_> {
        Box::new(
            self.iter()
                .filter_map(|(_k, square)| square.get_locations().ok())
                .flat_map(IntoIterator::into_iter),
        )
    }
}

/// Saves all occurences of every object id as a `json` file to the folder `out/data/rs3/locations`.
pub fn export_locations_by_id(config: &crate::cli::Config) -> CacheResult<()> {
    let out = path_macro::path!(config.output / "locations");

    fs::create_dir_all(&out)?;

    let last_id = {
        let squares = MapSquares::new(config)?.into_iter();
        squares
            .filter_map(|sq| sq.expect("error deserializing mapsquare").take_locations().ok())
            .filter(|locs| !locs.is_empty())
            .map(|locs| locs.last().expect("locations stopped existing").id)
            .max()
            .unwrap()
    };

    let squares = MapSquares::new(config)?.into_iter();
    let mut locs: Vec<_> = squares
        .filter_map(|sq| sq.expect("error deserializing mapsquare").take_locations().ok())
        .map(|locs| locs.into_iter().peekable())
        .collect();

    // Here we exploit the fact that the mapsquare file yields its locations by id in ascending order.
    (0..=last_id)
        .map(|id| {
            (
                id,
                locs.iter_mut()
                    .flat_map(|iterator| std::iter::repeat_with(move || iterator.next_if(|loc| loc.id == id)).take_while(|item| item.is_some()))
                    .flatten()
                    .collect::<Vec<Location>>(),
            )
        })
        .par_bridge()
        .for_each(|(id, id_locs)| {
            if !id_locs.is_empty() && id != 83 {
                let mut file = File::create(path!(&out / f!("{id}.json"))).unwrap();
                let data = serde_json::to_string_pretty(&id_locs).unwrap();
                file.write_all(data.as_bytes()).unwrap();
            }
        });

    Ok(())
}

/// Saves all occurences of every object id as a `json` file to the folder `out/data/rs3/locations`.
pub fn export_locations_by_square(config: &crate::cli::Config) -> CacheResult<()> {
    let out = path_macro::path!(config.output / "locations");

    fs::create_dir_all(&out)?;
    MapSquares::new(config)?.into_iter().par_bridge().for_each(|sq| {
        let sq = sq.expect("error deserializing mapsquare");
        let i = sq.i;
        let j = sq.j;
        if let Ok(locations) = sq.take_locations() {
            if !locations.is_empty() {
                let mut file = File::create(path!(&out / f!("{i}_{j}.json"))).unwrap();
                let data = serde_json::to_string_pretty(&locations).unwrap();
                file.write_all(data.as_bytes()).unwrap();
            }
        }
    });

    Ok(())
}

/// Saves all occurences of every object id as a `json` file to the folder `out/data/rs3/locations`.
pub fn export_tiles_by_square(config: &crate::cli::Config) -> CacheResult<()> {
    let out = path_macro::path!(config.output / "tiles");

    fs::create_dir_all(&out)?;
    MapSquares::new(config)?.into_iter().par_bridge().for_each(|sq| {
        let sq = sq.expect("error deserializing mapsquare");
        let i = sq.i;
        let j = sq.j;
        if let Ok(tiles) = sq.take_tiles() {
            if !tiles.is_empty() {
                let mut file = File::create(path!(&out / f!("{i}_{j}.json"))).unwrap();
                let data = serde_json::to_string_pretty(&tiles).unwrap();
                file.write_all(data.as_bytes()).unwrap();
            }
        }
    });

    Ok(())
}

#[cfg(all(test, any(feature = "rs3", feature = "osrs")))]
mod tests {
    use super::*;
    use crate::cli::Config;
    #[test]
    fn water() -> CacheResult<()> {
        let config = Config::env();

        let squares = MapSquares::new(&config)?.into_iter();
        for square in squares {
            let square = square.unwrap();
            if square.i() == 40 && square.j() == 62 {
                for i in 0..4 {
                    let tile = square.get_tiles()?.get([i, 10, 10]);
                    dbg!(tile);
                    return Ok(());
                }
            }
        }
        panic!("Unable to get some water");
    }
}

#[cfg(all(test, feature = "legacy"))]
mod legacy {
    use super::*;
    use crate::cli::Config;
    #[test]
    fn decode_50_50() {
        let config = Config::env();

        let mut cache = CacheIndex::new(4, &config.input).unwrap();
        let index = cache.get_index();
        let meta = index[&(50, 50)];
        dbg!(meta);
        let locs = cache.archive(meta.locfile as u32).unwrap().file(&0).unwrap();
        let map = cache.archive(meta.mapfile as u32).unwrap().file(&0).unwrap();
    }
}

#[cfg(all(test, feature = "legacy"))]
mod mapsquare_iter {

    use super::*;
    #[test]
    fn traverse() -> CacheResult<()> {
        let config = crate::cli::Config::env();
        let sqs = MapSquares::new(&config)?.into_iter();

        for sq in sqs {
            if let Ok(locations) = sq.expect("error deserializing mapsquare").get_locations() {
                for loc in locations {
                    if loc.id == 3263 {
                        println!("{} {} {} {}", loc.i, loc.i, loc.x, loc.y);
                        if loc.i == 56 && loc.j == 54 && loc.x == 62 && loc.y == 53 {
                            return Ok(());
                        }
                    }
                }
            }
        }
        panic!("swamp not found");
    }
}

#[cfg(all(test, feature = "rs3"))]
mod map_tests {
    use super::*;

    #[test]
    fn loc_0_50_50_9_16_is_trapdoor() -> CacheResult<()> {
        let config = crate::cli::Config::env();

        let id = 36687_u32;
        let square = MapSquare::new(50, 50, &config)?;
        assert!(square
            .get_locations()?
            .iter()
            .any(|loc| loc.id == id && loc.plane.matches(&0) && loc.x == 9 && loc.y == 16));
        Ok(())
    }

    #[test]
    fn get_tile() -> CacheResult<()> {
        let config = crate::cli::Config::env();

        let square = MapSquare::new(49, 54, &config)?;
        let _tile = square.get_tiles()?.get([0, 24, 25]);
        Ok(())
    }
}
