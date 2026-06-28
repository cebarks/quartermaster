use mini_moka::sync::Cache;

#[derive(Clone)]
pub struct ForgeResponseCache {
    inner: Cache<String, Vec<u8>>,
}

impl ForgeResponseCache {
    pub fn new(max_capacity: u64, ttl_secs: u64) -> Self {
        Self {
            inner: Cache::builder()
                .max_capacity(max_capacity)
                .time_to_live(std::time::Duration::from_secs(ttl_secs))
                .build(),
        }
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.inner.get(&key.to_string())
    }

    pub fn insert(&self, key: String, value: Vec<u8>) {
        self.inner.insert(key, value);
    }

    #[allow(dead_code)] // exposed as public API via ForgeClient::invalidate_cache
    pub fn invalidate_all(&self) {
        self.inner.invalidate_all();
    }
}
