//! Resource limits enforced while opening a container (SPEC §2.3).

/// Decompression and structure limits applied by [`crate::open_with_limits`].
///
/// The defaults implement the SPEC §2.3 zip-bomb defence: a 100:1 per-entry
/// decompression ratio cap and an absolute uncompressed size cap. All limits
/// are configurable because readers on constrained platforms (mobile, web)
/// may want tighter budgets, and tests want tiny ones.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Total uncompressed bytes allowed for the whole container, shared with
    /// every nested archive. This is the hard memory line. Default: 1 GiB.
    pub max_total_uncompressed: u64,
    /// Maximum uncompressed/compressed ratio per entry (SPEC §2.3 recommends
    /// 100:1). Default: 100.
    pub max_compression_ratio: u64,
    /// Entries at or below this uncompressed size never trip the ratio check:
    /// a tiny, highly compressible file is not a bomb. Default: 1 MiB.
    pub ratio_grace_bytes: u64,
    /// Maximum number of entries in an archive. Default: 10 000.
    pub max_entries: usize,
    /// Maximum archive nesting depth. Depth 1 is the container itself; Python
    /// wheels in `deps/` are ZIPs at depth 2 and may themselves carry ZIP data
    /// at depth 3. A shared decompression budget makes deeper recursion safe
    /// to refuse outright ("zip quine" defence). Default: 3.
    pub max_nesting_depth: u8,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_total_uncompressed: 1024 * 1024 * 1024,
            max_compression_ratio: 100,
            ratio_grace_bytes: 1024 * 1024,
            max_entries: 10_000,
            max_nesting_depth: 3,
        }
    }
}
