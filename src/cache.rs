use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

/// Get the global cache directory for the current project.
/// Returns: ~/.cache/taut/<project-hash>/ (platform-specific)
pub fn get_cache_dir() -> PathBuf {
    let cache_base = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("taut");

    // Hash the current directory to isolate per-project caches
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(cwd.to_string_lossy().as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let project_hash = &hash[..16]; // First 16 chars

    cache_base.join(project_hash)
}

/// Ensure the cache directory exists
pub fn ensure_cache_dir() -> std::io::Result<PathBuf> {
    let dir = get_cache_dir();
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Clear all caches for the current project
/// Returns the stats of what was cleared (size_bytes, file_count)
pub fn clear_cache() -> std::io::Result<(u64, usize)> {
    let stats = get_cache_stats();
    let dir = get_cache_dir();
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    Ok((stats.size_bytes, stats.file_count))
}

/// Get cache statistics
pub struct CacheStats {
    pub cache_dir: PathBuf,
    pub exists: bool,
    pub size_bytes: u64,
    pub file_count: usize,
}

pub fn get_cache_stats() -> CacheStats {
    let cache_dir = get_cache_dir();
    let exists = cache_dir.exists();

    let (size_bytes, file_count) = if exists {
        walkdir::WalkDir::new(&cache_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .fold((0u64, 0usize), |(size, count), entry| {
                let file_size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                (size + file_size, count + 1)
            })
    } else {
        (0, 0)
    };

    CacheStats {
        cache_dir,
        exists,
        size_bytes,
        file_count,
    }
}
