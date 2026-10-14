use bincode::{Decode, Encode};
use jgenesis_common::frontend::SaveWriter;
use std::collections::HashMap;
use std::path::PathBuf;

/// Battery-backed SRAM and other per-game persistence requested by the core.
pub trait SaveStore: Send {
    /// Returns `Ok(None)` when there is no saved data for `ext`.
    ///
    /// # Errors
    ///
    /// Returns an error if saved data may exist but cannot be read. Callers must not treat that as
    /// "no save", or they would later overwrite the real data with blank data.
    fn load(&mut self, ext: &str) -> std::io::Result<Option<Vec<u8>>>;
    /// # Errors
    ///
    /// Returns an error if the bytes cannot be stored.
    fn persist(&mut self, ext: &str, bytes: &[u8]) -> std::io::Result<()>;
}

#[derive(Default)]
pub struct MemorySaveStore(pub HashMap<String, Vec<u8>>);

impl SaveStore for MemorySaveStore {
    fn load(&mut self, ext: &str) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.0.get(ext).cloned())
    }
    fn persist(&mut self, ext: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.0.insert(ext.to_owned(), bytes.to_vec());
        Ok(())
    }
}

pub struct FsSaveStore {
    dir: PathBuf,
    stem: String,
}

impl FsSaveStore {
    /// # Errors
    ///
    /// Returns an error if the save directory cannot be created.
    pub fn new(dir: PathBuf, stem: impl Into<String>) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir, stem: stem.into() })
    }
    fn path(&self, ext: &str) -> PathBuf {
        self.dir.join(format!("{}.{ext}", self.stem))
    }
}

impl SaveStore for FsSaveStore {
    fn load(&mut self, ext: &str) -> std::io::Result<Option<Vec<u8>>> {
        match std::fs::read(self.path(ext)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn persist(&mut self, ext: &str, bytes: &[u8]) -> std::io::Result<()> {
        let tmp = self.path(&format!("{ext}.tmp"));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(tmp, self.path(ext))
    }
}

/// Adapts a `SaveStore` to jgenesis' `SaveWriter`.
pub(crate) struct SaveWriterAdapter<'a>(pub &'a mut dyn SaveStore);

impl SaveWriter for SaveWriterAdapter<'_> {
    type Err = std::io::Error;

    fn load_bytes(&mut self, extension: &str) -> Result<Vec<u8>, Self::Err> {
        self.0.load(extension)?.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
    }
    fn persist_bytes(&mut self, extension: &str, bytes: &[u8]) -> Result<(), Self::Err> {
        self.0.persist(extension, bytes)
    }
    fn load_serialized<D: Decode<()>>(&mut self, extension: &str) -> Result<D, Self::Err> {
        let bytes = self.load_bytes(extension)?;
        bincode::decode_from_slice(&bytes, bincode::config::standard())
            .map(|(v, _)| v)
            .map_err(std::io::Error::other)
    }
    fn persist_serialized<E: Encode>(&mut self, extension: &str, data: E) -> Result<(), Self::Err> {
        let bytes = bincode::encode_to_vec(data, bincode::config::standard())
            .map_err(std::io::Error::other)?;
        self.persist_bytes(extension, &bytes)
    }
}
