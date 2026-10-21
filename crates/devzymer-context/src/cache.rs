use devzymer_core::store::Store;
use devzymer_core::Result;

/// Revision aware cache for retrieval results and session context.
pub struct ContextCache<'a> {
    store: &'a Store,
    ttl_secs: u64,
}

impl<'a> ContextCache<'a> {
    pub fn new(store: &'a Store, ttl_secs: u64) -> Self {
        Self { store, ttl_secs }
    }

    fn key(query: &str, level: u8) -> String {
        let hash = blake3::hash(format!("{query}|{level}").as_bytes());
        hash.to_hex()[..24].to_string()
    }

    pub fn get(&self, query: &str, level: u8) -> Result<Option<String>> {
        let revision = self.store.revision()?;
        let key = Self::key(query, level);
        match self.store.cache_get(&key, revision)? {
            Some(payload) => {
                let created = self.created_at(&key)?;
                let age = devzymer_core::util::now_millis().saturating_sub(created);
                if self.ttl_secs > 0 && age > self.ttl_secs * 1000 {
                    return Ok(None);
                }
                Ok(Some(payload))
            }
            None => Ok(None),
        }
    }

    pub fn put(&self, query: &str, level: u8, payload: &str) -> Result<()> {
        let revision = self.store.revision()?;
        self.store
            .cache_put(&Self::key(query, level), revision, payload)
    }

    fn created_at(&self, key: &str) -> Result<u64> {
        let _ = key;
        Ok(devzymer_core::util::now_millis())
    }

    pub fn clear(&self) -> Result<()> {
        self.store.cache_clear()
    }
}
