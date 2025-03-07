use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env::{self, VarError},
    fs::{self, File},
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    marker::PhantomData,
    ops::RangeInclusive,
    path::{Path, PathBuf},
    sync::Arc,
};

use bytes::{Buf, Bytes};
use itertools::iproduct;
use path_macro::path;

use crate::{
    arc::Archive,
    buf::BufExtra,
    decoder,
    error::{CacheError, CacheResult},
    index::{CacheIndex, CachePath, IndexState, Initial},
    meta::{IndexMetadata, Metadata},
};

impl<S> CacheIndex<S>
where
    S: IndexState,
{
    /// Loads the [`Metadata`] of `self`.
    fn get_raw_metadata(connection: &rusqlite::Connection) -> CacheResult<Bytes> {
        let mut stmt = connection.prepare("SELECT DATA FROM cache_index")?;
        let mut rows = stmt.query([])?;
        let data = rows.next()?.unwrap().get(0).unwrap();

        Ok(decoder::decompress(data)?)
    }

    /// Executes a sql command to retrieve an archive from the cache.
    pub fn get_file(&self, metadata: &Metadata) -> CacheResult<Bytes> {
        let mut stmt = self.connection.prepare("SELECT DATA, CRC, VERSION FROM cache WHERE KEY=?")?;
        let mut rows = stmt.query([metadata.archive_id()])?;
        let row = rows
            .next()?
            .ok_or_else(|| CacheError::archive_missing(self.index_id, metadata.archive_id()))?;
        let data = row.get(0).unwrap();
        let crc = row.get(1).unwrap();
        let version = row.get(2).unwrap();

        // wut
        let crc_offset = match self.index_id() {
            8 => 2_i64,
            47 => 2_i64,
            _ => 1_i64,
        };

        if crc == 0 && version == 0 {
            Err(CacheError::archive_missing(metadata.index_id(), metadata.archive_id()))
        } else if metadata.crc() as i64 + crc_offset != crc {
            Err(CacheError::crc(
                metadata.index_id(),
                metadata.archive_id(),
                metadata.crc() as i64 + crc_offset,
                crc,
            ))
        } else if metadata.version() as i64 != version {
            Err(CacheError::version(
                metadata.index_id(),
                metadata.archive_id(),
                metadata.version() as i64,
                version,
            ))
        } else {
            Ok(decoder::decompress(data)?)
        }
    }

    /// Assert whether the cache held by `self` is in a coherent state.
    ///
    /// # Errors
    ///
    /// May raise [`CrcError`](CacheError::CrcError), [`VersionError`](CacheError::VersionError) or [`ArchiveNotFoundError`](CacheError::ArchiveNotFoundError)
    /// if the cache is not in a logical state.
    ///
    /// # Notes
    /// Indices `VORBIS`, `AUDIOSTREAMS`, `TEXTURES_PNG_MIPPED` and `TEXTURES_ETC` tend to never complete.
    /// For these, simply ignore [`ArchiveNotFoundError`](CacheError::ArchiveNotFoundError).
    pub fn assert_coherence(&self) -> CacheResult<()> {
        for (archive_id, metadata) in self.metadatas().iter() {
            let mut stmt = self.connection.prepare("SELECT CRC, VERSION FROM cache WHERE KEY=?")?;
            let mut rows = stmt.query([archive_id])?;
            let row = rows.next()?.unwrap();
            let crc = row.get(0).unwrap();
            let version = row.get(1).unwrap();

            // wut
            let crc_offset = match self.index_id() {
                8 => 2_i64,
                47 => 2_i64,
                _ => 1_i64,
            };
            if crc == 0 && version == 0 {
                return Err(CacheError::archive_missing(metadata.index_id(), metadata.archive_id()));
            } else if metadata.crc() as i64 + crc_offset != crc {
                return Err(CacheError::crc(
                    metadata.index_id(),
                    metadata.archive_id(),
                    metadata.crc() as i64 + crc_offset,
                    crc,
                ));
            } else if metadata.version() as i64 != version {
                return Err(CacheError::version(
                    metadata.index_id(),
                    metadata.archive_id(),
                    metadata.version() as i64,
                    version,
                ));
            }
        }
        Ok(())
    }
}

impl CacheIndex<Initial> {
    /// Constructor for [`CacheIndex`].
    ///
    /// # Errors
    ///
    /// Raises [`CacheNotFoundError`](CacheError::CacheNotFoundError) if the cache database cannot be found.
    pub fn new(index_id: u32, path: Arc<CachePath>) -> CacheResult<CacheIndex<Initial>> {
        let file = path!(path.as_ref() / format!("js5-{index_id}.jcache"));

        // check if database exists (without creating blank sqlite databases)
        match fs::metadata(&file) {
            Ok(_) => {
                let connection = rusqlite::Connection::open(file)?;
                let raw_metadata: Bytes = Self::get_raw_metadata(&connection)?;
                let metadatas = IndexMetadata::deserialize(index_id, raw_metadata)?;

                Ok(Self {
                    index_id,
                    metadatas,
                    connection,
                    path,
                    state: Initial {},
                })
            }
            Err(e) => Err(CacheError::cache_not_found(e, file, path)),
        }
    }
}

/// Asserts whether all indices' metadata match their contents.
/// Indices 14, 40, 54, 55 are not necessarily complete.
///
/// Exposed as `--assert coherence`.
///
/// # Panics
///
/// Panics if compiled with feature `mockdata`.
#[cfg(not(feature = "mockdata"))]
pub fn assert_coherence(folder: Arc<CachePath>) -> CacheResult<()> {
    for index_id in 0..70 {
        if fs::metadata(path!(&*folder / format!("js5-{index_id}.jcache"))).is_ok() {
            match CacheIndex::new(index_id, folder.clone())?.assert_coherence() {
                Ok(_) => println!("Index {index_id} is coherent!"),
                Err(e) => println!("Index {index_id} is not coherent: {e} and possibly others."),
            }
        }
    }
    Ok(())
}
