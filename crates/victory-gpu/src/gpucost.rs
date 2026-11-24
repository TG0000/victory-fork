//! What the card's time goes to, by command (`VICTORY_GPU_COST=1`,
//! diagnostics builds; off by default).
//!
//! A GPU timestamp goes before each noted command of a title list and at the
//! list's end, and around our own lists (state preludes, the passes over
//! copies in the card's memory). Submissions stay as they are: unlike
//! `VICTORY_GPU_TIMING` nothing is cut or waited for. A list's timestamps are
//! read when the list is next reset, which is after the card ran it, so a line
//! covers lists of a moment before. Timestamps are written when all earlier
//! work is done: a command's time is how much later it was done than the one
//! before it, waits on barriers between them included.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use windows::Win32::Graphics::Direct3D12::*;

/// Whether the report was asked for.
pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_GPU_COST").is_some_and(|v| v != "0"))
}

/// Times of commands (count, ticks) by what they run and, in a second table,
/// by what they draw into; the spans of lists on the card's clock.
#[derive(Default)]
struct Sums {
    by_work: HashMap<String, (u64, u64)>,
    by_pass: HashMap<String, (u64, u64)>,
    spans: Vec<(u64, u64)>,
}

static SUMS: Mutex<Option<Sums>> = Mutex::new(None);

/// Commands beyond a list's timestamps, not timed.
pub(crate) static UNTIMED: AtomicU64 = AtomicU64::new(0);

fn add(map: &mut HashMap<String, (u64, u64)>, key: String, ticks: u64) {
    let e = map.entry(key).or_insert((0, 0));
    e.0 += 1;
    e.1 += ticks;
}

/// A note's parts: what it is summed under by work, and by pass. A note is
/// `kind size [pipeline] into`, or words without a pipeline.
fn keys(note: &str) -> (String, String) {
    let word = note.split(' ').next().unwrap_or("");
    match (note.find('['), note.find(']')) {
        (Some(a), Some(b)) if a < b => {
            let into = note[b + 1..].trim();
            let pass = if into.is_empty() { word.to_owned() } else { format!("into {into}") };
            (format!("{word} {}", &note[a..=b]), pass)
        }
        _ => {
            let key = note.split(' ').take(2).collect::<Vec<_>>().join(" ");
            (key.clone(), key)
        }
    }
}

/// A list ran: its notes, and the timestamps before each and at its end.
pub(crate) fn ran(notes: &[&str], stamps: &[u64]) {
    let (Some(&first), Some(&last)) = (stamps.first(), stamps.last()) else {
        return;
    };
    let mut sums = SUMS.lock().unwrap_or_else(|e| e.into_inner());
    let sums = sums.get_or_insert_with(Sums::default);
    for (note, pair) in notes.iter().zip(stamps.windows(2)) {
        let ticks = pair[1].saturating_sub(pair[0]);
        let (work, pass) = keys(note);
        add(&mut sums.by_work, work, ticks);
        add(&mut sums.by_pass, pass, ticks);
    }
    if last > first {
        sums.spans.push((first, last));
    }
}

/// Timestamp pairs around our own lists.
struct Ring {
    heap: ID3D12QueryHeap,
    back: ID3D12Resource,
    data: usize,
    next: u32,
    /// Pairs not read yet: index, what they are around, when recorded.
    out: std::collections::VecDeque<(u32, &'static str, std::time::Instant)>,
}

// SAFETY: free-threaded objects behind a lock.
unsafe impl Send for Ring {}

const PAIRS: u32 = 16384;
static RING: Mutex<Option<Ring>> = Mutex::new(None);

/// Begins timing one of our lists (`what`), just opened; [`end`] gets the
/// result just before it is closed.
pub(crate) fn begin(list: &ID3D12GraphicsCommandList, what: &'static str) -> Option<u32> {
    if !on() {
        return None;
    }
    let mut ring = RING.lock().unwrap_or_else(|e| e.into_inner());
    if ring.is_none() {
        let (heap, back) = crate::host::timestamps(2 * PAIRS)?;
        let mut data = std::ptr::null_mut();
        // SAFETY: a read-back buffer, mapped for good.
        unsafe { back.Map(0, None, Some(&mut data)).ok()? };
        *ring = Some(Ring { heap, back, data: data as usize, next: 0, out: Default::default() });
    }
    let ring = ring.as_mut()?;
    let at = ring.next % PAIRS;
    ring.next = ring.next.wrapping_add(1);
    while ring.out.len() >= PAIRS as usize / 2 {
        ring.out.pop_front();
    }
    ring.out.push_back((at, what, std::time::Instant::now()));
    // SAFETY: an open list and our heap.
    unsafe { list.EndQuery(&ring.heap, D3D12_QUERY_TYPE_TIMESTAMP, 2 * at) };
    Some(at)
}

/// Ends what [`begin`] began.
pub(crate) fn end(list: &ID3D12GraphicsCommandList, at: Option<u32>) {
    let Some(at) = at else { return };
    let ring = RING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(ring) = ring.as_ref() else { return };
    // SAFETY: an open list, our heap and read-back buffer.
    unsafe {
        list.EndQuery(&ring.heap, D3D12_QUERY_TYPE_TIMESTAMP, 2 * at + 1);
        list.ResolveQueryData(
            &ring.heap,
            D3D12_QUERY_TYPE_TIMESTAMP,
            2 * at,
            2,
            &ring.back,
            u64::from(at) * 16,
        );
    }
}

/// Reads the pairs of our lists that ran a while ago.
fn read_ours() {
    let mut read = Vec::new();
    {
        let mut ring = RING.lock().unwrap_or_else(|e| e.into_inner());
        let Some(ring) = ring.as_mut() else { return };
        while let Some(&(at, what, when)) = ring.out.front() {
            if when.elapsed() < std::time::Duration::from_secs(3) {
                break;
            }
            ring.out.pop_front();
            // SAFETY: the pair lies in the mapped buffer; the list ran
            // seconds ago.
            let pair = unsafe { *((ring.data + at as usize * 16) as *const [u64; 2]) };
            read.push((what, pair));
        }
    }
    for (what, pair) in read {
        ran(&[what], &pair);
    }
}

/// The time spans cover together, and the gaps between them by length
/// (under 0.05 ms, under 1 ms, longer: count and ticks each).
fn cover(spans: &mut [(u64, u64)], frequency: u64) -> (u64, [(u64, u64); 3]) {
    spans.sort_unstable();
    let (mut busy, mut gaps, mut end) = (0, [(0, 0); 3], 0);
    for (i, &(from, to)) in spans.iter().enumerate() {
        if i == 0 {
            end = from;
        }
        if from > end {
            let gap = from - end;
            let place = match gap * 100_000 / frequency.max(1) {
                0..5 => 0,
                5..100 => 1,
                _ => 2,
            };
            gaps[place].0 += 1;
            gaps[place].1 += gap;
            end = from;
        }
        if to > end {
            busy += to - end;
            end = to;
        }
    }
    (busy, gaps)
}

/// For the frame-rate line: what the lists read since the last one took,
/// per frame of `frames`.
pub(crate) fn report(frames: f64, frequency: u64) {
    if !on() || frames <= 0.0 {
        return;
    }
    read_ours();
    let Some(mut sums) = SUMS.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    let ms = |ticks: u64| ticks as f64 * 1e3 / frequency.max(1) as f64 / frames;
    let (busy, gaps) = cover(&mut sums.spans, frequency);
    let per = |n: u64| n as f64 / frames;
    tracing::info!(
        "gpu cost per frame: {:.0} lists, busy {:.1} ms; between them {:.0} gaps under 0.05 ms {:.1} ms, \
         {:.0} under 1 ms {:.1} ms, {:.1} longer {:.1} ms; {:.0} commands not timed",
        per(sums.spans.len() as u64),
        ms(busy),
        per(gaps[0].0),
        ms(gaps[0].1),
        per(gaps[1].0),
        ms(gaps[1].1),
        per(gaps[2].0),
        ms(gaps[2].1),
        per(UNTIMED.swap(0, Ordering::Relaxed)),
    );
    for (title, map, most) in [("work", sums.by_work, 40), ("pass", sums.by_pass, 25)] {
        let total: u64 = map.values().map(|v| v.1).sum();
        let mut all: Vec<_> = map.into_iter().collect();
        all.sort_by_key(|(_, (_, ticks))| std::cmp::Reverse(*ticks));
        tracing::info!("gpu cost by {title}: {} kinds, {:.1} ms per frame", all.len(), ms(total));
        for (key, (n, ticks)) in all.into_iter().take(most) {
            tracing::info!("gpu cost {title} {:.2} ms {:.1}x {key}", ms(ticks), per(n));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_summed_by_pipeline_and_by_target() {
        let (work, pass) = keys("draw 3x1 [vertex a, pixel b] 1920x1080 f10");
        assert_eq!(work, "draw [vertex a, pixel b]");
        assert_eq!(pass, "into 1920x1080 f10");
        let (work, pass) = keys("dispatch 8x8x1 [compute c]");
        assert_eq!(work, "dispatch [compute c]");
        assert_eq!(pass, "dispatch");
        assert_eq!(keys("copy resource 0x1 to 0x2").0, "copy resource");
    }

    #[test]
    fn overlapping_spans_count_once_and_gaps_by_length() {
        // A clock of 100,000 ticks per second: one tick is 0.01 ms.
        let mut spans = vec![(1000, 1200), (100, 200), (150, 300), (302, 400), (410, 500)];
        let (busy, gaps) = cover(&mut spans, 100_000);
        assert_eq!(busy, 200 + 98 + 90 + 200);
        assert_eq!(gaps, [(1, 2), (1, 10), (1, 500)]);
    }
}
