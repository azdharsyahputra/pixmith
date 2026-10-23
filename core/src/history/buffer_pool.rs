//! Reuse of large pixel buffers.
//!
//! Editing a cel that an undo snapshot still shares copies it first (copy-on-write). For big
//! canvases most of that cost is not the copy but faulting in a freshly mapped allocation, so
//! buffers that history compaction frees are kept here and handed to the next copy of the same
//! size, which is then done by several threads.

use std::mem::MaybeUninit;
use std::sync::Mutex;

/// Smaller buffers are cheap to allocate and copy; they bypass the pool.
const MIN_BYTES: usize = 1 << 20;
/// Upper bound on memory parked in the pool.
const MAX_POOLED_BYTES: usize = 192 << 20;
/// Bytes each copying thread handles at least.
const BYTES_PER_THREAD: usize = 4 << 20;

static POOL: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

/// Parks a buffer nobody uses any more so a later copy of the same size can reuse its memory.
pub(crate) fn recycle(v: Vec<u8>) {
    if v.capacity() < MIN_BYTES || v.capacity() > MAX_POOLED_BYTES {
        return;
    }
    let Ok(mut pool) = POOL.lock() else { return };
    let mut total = v.capacity() + pool.iter().map(Vec::capacity).sum::<usize>();
    while total > MAX_POOLED_BYTES && !pool.is_empty() {
        total -= pool.remove(0).capacity();
    }
    pool.push(v);
}

/// A pooled buffer with room for `len` bytes and not much more, if one is parked.
fn take(len: usize) -> Option<Vec<u8>> {
    let mut pool = POOL.lock().ok()?;
    let i = pool.iter().position(|v| v.capacity() >= len && v.capacity() - len <= len / 4)?;
    Some(pool.swap_remove(i))
}

/// A copy of `src`; large ones reuse pooled memory and are copied by several threads.
pub(crate) fn copy_of(src: &[u8]) -> Vec<u8> {
    if src.len() < MIN_BYTES {
        return src.to_vec();
    }
    let mut dst = take(src.len()).unwrap_or_else(|| Vec::with_capacity(src.len()));
    dst.clear();
    copy_into(src, &mut dst.spare_capacity_mut()[..src.len()]);
    // SAFETY: `copy_into` initialized the first `src.len()` bytes.
    unsafe { dst.set_len(src.len()) };
    dst
}

fn copy_into(src: &[u8], dst: &mut [MaybeUninit<u8>]) {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    let chunk = src.len().div_ceil(threads).max(BYTES_PER_THREAD);
    let copy = |s: &[u8], d: &mut [MaybeUninit<u8>]| {
        // SAFETY: equal lengths, distinct allocations; u8 needs no drop.
        unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), d.as_mut_ptr().cast::<u8>(), s.len()) }
    };
    if chunk >= src.len() {
        return copy(src, dst);
    }
    std::thread::scope(|scope| {
        for (s, d) in src.chunks(chunk).zip(dst.chunks_mut(chunk)) {
            scope.spawn(move || copy(s, d));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_are_exact_for_small_and_large_buffers() {
        for len in [0, 7, MIN_BYTES - 1, MIN_BYTES, 3 * BYTES_PER_THREAD + 13] {
            let src: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
            assert_eq!(copy_of(&src), src, "len {len}");
        }
    }

    #[test]
    fn recycled_buffers_are_reused_and_fully_overwritten() {
        let len = 2 * MIN_BYTES;
        let old = vec![0xAB; len];
        let ptr = old.as_ptr();
        recycle(old);
        let src = vec![0x11; len];
        let copy = copy_of(&src);
        assert_eq!(copy, src);
        // Another test may have taken it meanwhile; when it was reused, no old byte survives.
        if copy.as_ptr() == ptr {
            assert!(copy.iter().all(|&b| b == 0x11));
        }
    }
}
