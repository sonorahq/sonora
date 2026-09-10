use std::time::Duration;

use gpui::App;
use state::Sonora;

const INTERVAL: Duration = Duration::from_secs(30);

pub fn watch(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(INTERVAL).await;
            if log::log_enabled!(log::Level::Debug) {
                let probed = cx
                    .background_executor()
                    .spawn(async { (footprint().unwrap_or_default(), resident()) })
                    .await;
                cx.update(|cx| report(probed, cx));
            }
            cx.background_executor().spawn(async { release() }).detach();
        }
    })
    .detach();
}

fn report(probed: (Footprint, Option<usize>), cx: &mut App) {
    let (footprint, resident) = probed;
    let (entries, bytes) = ui::artwork_usage(cx).unwrap_or((0, 0));
    let queue = Sonora::global(cx).queue.read(cx);
    let past = queue.past().len();
    let ahead = queue.upcoming().len() + queue.similar().len();

    log::debug!(
        "memory: rss {}, heap {}, gpu {}, file {}, artwork {entries} entries / {}, queue {past} past / {ahead} ahead",
        resident.map_or_else(|| "unknown".to_owned(), mib),
        mib(footprint.heap),
        mib(footprint.gpu),
        mib(footprint.file),
        mib(bytes)
    );
}

#[derive(Default)]
struct Footprint {
    heap: usize,
    gpu: usize,
    file: usize,
}

#[cfg(target_os = "linux")]
enum Bucket {
    Heap,
    Gpu,
    File,
}

#[cfg(target_os = "linux")]
fn footprint() -> Option<Footprint> {
    Some(tally(&std::fs::read_to_string("/proc/self/smaps").ok()?))
}

#[cfg(target_os = "linux")]
fn tally(smaps: &str) -> Footprint {
    const GPU: [&str; 6] = [
        "/dev/dri",
        "/dev/nvidia",
        "memfd:",
        "dmabuf",
        "/SYSV",
        "amdgpu",
    ];

    let mut footprint = Footprint::default();
    let mut bucket = Bucket::Heap;

    for line in smaps.lines() {
        if let Some(path) = mapping(line) {
            bucket = match path {
                _ if GPU.iter().any(|kind| path.contains(kind)) => Bucket::Gpu,
                "" => Bucket::Heap,
                _ if path.starts_with('[') => Bucket::Heap,
                _ => Bucket::File,
            };
            continue;
        }
        if let Some(rest) = line.strip_prefix("Rss:") {
            let bytes = rest
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0)
                * 1024;

            match bucket {
                Bucket::Heap => footprint.heap += bytes,
                Bucket::Gpu => footprint.gpu += bytes,
                Bucket::File => footprint.file += bytes,
            }
        }
    }

    footprint
}

#[cfg(target_os = "linux")]
fn mapping(line: &str) -> Option<&str> {
    let mut fields = line.split_whitespace();
    let range = fields.next()?;
    let perms = fields.next()?;
    if !range.contains('-') || perms.len() != 4 {
        return None;
    }
    if !perms.ends_with('p') && !perms.ends_with('s') {
        return None;
    }

    Some(fields.nth(3).unwrap_or(""))
}

#[cfg(not(target_os = "linux"))]
fn footprint() -> Option<Footprint> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::tally;

    const SMAPS: &str = "\
55d0f4a00000-55d0f4a21000 rw-p 00000000 00:00 0                          [heap]
Rss:                 512 kB
7f9c00000000-7f9c04000000 rw-p 00000000 00:00 0
Rss:                2048 kB
VmFlags: rd wr mr mw me ac
7f9c10000000-7f9c10800000 rw-s 00000000 00:0f 1234       /dev/dri/renderD128
Rss:                4096 kB
7f9c20000000-7f9c20100000 rw-s 00000000 00:01 4321       /memfd:wayland-shm (deleted)
Rss:                1024 kB
7f9c30000000-7f9c30200000 r--p 00000000 103:02 99        /nix/store/libfoo.so
Rss:                 256 kB
7ffd12300000-7ffd12321000 rw-p 00000000 00:00 0          [stack]
Rss:                 128 kB
";

    #[test]
    fn every_mapping_lands_in_a_bucket() {
        let footprint = tally(SMAPS);

        assert_eq!(footprint.heap, (512 + 2048 + 128) * 1024);
        assert_eq!(footprint.gpu, (4096 + 1024) * 1024);
        assert_eq!(footprint.file, 256 * 1024);
    }

    #[test]
    fn counters_ignore_lines_that_only_look_like_mappings() {
        let footprint = tally("VmFlags: rd wr mr mw me ac\nRss:  64 kB\n");

        assert_eq!(footprint.heap, 64 * 1024);
        assert_eq!(footprint.gpu, 0);
        assert_eq!(footprint.file, 0);
    }
}

fn mib(bytes: usize) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024. * 1024.))
}

#[cfg(not(target_env = "msvc"))]
fn release() {
    unsafe {
        tikv_jemalloc_sys::mallctl(
            c"arena.4096.purge".as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        );
    }
}

#[cfg(target_env = "msvc")]
fn release() {}

#[cfg(target_os = "linux")]
fn resident() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kib: usize = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

#[cfg(not(target_os = "linux"))]
fn resident() -> Option<usize> {
    None
}
