## MODIFIED Requirements

### Requirement: Cache is bounded and regenerable
Thumbnails SHALL be cached in memory in a byte-capped LRU (128 MiB by default, configurable from 32 to 2048 MiB) and on disk as PNG files in the platform cache directory (`<cache>/thumbs`), trimmed to 256 MiB on first use, keyed by uri + mtime + size + edge and versioned so a format change invalidates old files. `preview.thumbs.clear_cache` SHALL drop both memory and disk copies and re-request the visible window. A stale mtime/size SHALL miss the cache rather than show an outdated thumbnail.

#### Scenario: Cache size cap
- **WHEN** the disk cache exceeds 256 MiB
- **THEN** it is trimmed to the cap on next use and the newest files are kept

#### Scenario: Edited file
- **WHEN** a cached image is modified on disk and previewed again
- **THEN** the thumbnail is regenerated (the cache key includes mtime and size)

#### Scenario: Memory cache default
- **WHEN** the preferences do not set `thumb_memory_mib`
- **THEN** decoded thumbnails use at most 128 MiB of memory, the least recently drawn going first
