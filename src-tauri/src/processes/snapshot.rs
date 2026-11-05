//! One `NtQuerySystemInformation(SystemProcessInformation)` call: every process's private working set (Task
//! Manager's "Memory" column), creation time, parent pid, session and image name, without opening any process.
//! Measured at ~3.5 ms for 339 processes on the dev PC (Phase 3 design).
//!
//! The windows crate hides the fields we need in `Reserved*` arrays, so the buffer is read at fixed x64 offsets
//! (phnt's `SYSTEM_PROCESS_INFORMATION`, stable since Vista). `parse` is pure and tested on a synthetic buffer.

use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Win32::Foundation::STATUS_INFO_LENGTH_MISMATCH;

/// A process as the kernel reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawProcess {
    pub pid: u32,
    pub parent: u32,
    /// FILETIME ticks (100 ns since 1601), as `GetProcessTimes` reports them.
    pub created: u64,
    pub session: u32,
    /// The image file name (`chrome.exe`); empty for the idle process.
    pub name: String,
    /// Private working set in bytes.
    pub private_bytes: u64,
}

// x64 offsets into one SYSTEM_PROCESS_INFORMATION entry.
const NEXT_ENTRY: usize = 0;
const WORKING_SET_PRIVATE: usize = 8;
const CREATE_TIME: usize = 32;
const IMAGE_NAME_LENGTH: usize = 56;
const IMAGE_NAME_BUFFER: usize = 64;
const PROCESS_ID: usize = 80;
const PARENT_ID: usize = 88;
const SESSION_ID: usize = 100;
/// The fixed part of an entry (up to the thread array), so every field above is in bounds.
pub const ENTRY_SIZE: usize = 256;

const INITIAL_BUFFER: usize = 1 << 20;
const MAX_BUFFER: usize = 64 << 20;

#[cfg(target_pointer_width = "64")]
pub fn read() -> Option<Vec<RawProcess>> {
    let mut len = INITIAL_BUFFER;
    loop {
        // u64 elements keep the buffer 8-byte aligned, as the kernel expects.
        let mut buf = vec![0u64; len.div_ceil(8)];
        let bytes = buf.len() * 8;
        let mut needed = 0u32;
        // SAFETY: `buf` is a writable buffer of `bytes` bytes and `needed` a valid out-pointer.
        let status = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                buf.as_mut_ptr().cast(),
                bytes as u32,
                &mut needed,
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH && bytes < MAX_BUFFER {
            // Processes start between calls; leave some room.
            len = (needed as usize + (64 << 10))
                .max(bytes * 2)
                .min(MAX_BUFFER);
            continue;
        }
        if status.is_err() {
            log::warn!("NtQuerySystemInformation failed: {status:?}");
            return None;
        }
        // SAFETY: reinterpreting the initialised u64 buffer as bytes; same allocation and length.
        let raw = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), bytes) };
        return Some(parse(raw, raw.as_ptr() as usize));
    }
}

/// Walks the entries in `buf`, which the kernel wrote at address `base` (image names point into it).
/// Stops at the first entry that doesn't fit, rather than reading out of bounds.
pub fn parse(buf: &[u8], base: usize) -> Vec<RawProcess> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at
        .checked_add(ENTRY_SIZE)
        .is_some_and(|end| end <= buf.len())
    {
        let entry = &buf[at..at + ENTRY_SIZE];
        let next = u32_at(entry, NEXT_ENTRY) as usize;
        let name_len = u16_at(entry, IMAGE_NAME_LENGTH) as usize;
        let name_ptr = u64_at(entry, IMAGE_NAME_BUFFER) as usize;
        let name = name_ptr
            .checked_sub(base)
            .and_then(|start| Some((start, start.checked_add(name_len)?)))
            .filter(|&(start, end)| name_len > 0 && end <= buf.len() && start % 2 == 0)
            .map(|(start, end)| {
                let (pairs, _) = buf[start..end].as_chunks::<2>();
                let units: Vec<u16> = pairs.iter().map(|&c| u16::from_le_bytes(c)).collect();
                String::from_utf16_lossy(&units)
            })
            .unwrap_or_default();
        out.push(RawProcess {
            pid: u64_at(entry, PROCESS_ID) as u32,
            parent: u64_at(entry, PARENT_ID) as u32,
            created: u64_at(entry, CREATE_TIME),
            session: u32_at(entry, SESSION_ID),
            name,
            private_bytes: u64_at(entry, WORKING_SET_PRIVATE),
        });
        if next == 0 {
            break;
        }
        at += next;
    }
    out
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap_or_default())
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a buffer of entries laid out like the kernel's, names stored after each entry.
    fn synthetic(procs: &[(u32, u32, u64, u32, &str, u64)], base: usize) -> Vec<u8> {
        let mut buf = Vec::new();
        for (i, &(pid, parent, created, session, name, private)) in procs.iter().enumerate() {
            let start = buf.len();
            let name16: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let size = ENTRY_SIZE + name16.len().next_multiple_of(8);
            buf.resize(start + size, 0);
            let e = &mut buf[start..start + size];
            let last = i + 1 == procs.len();
            e[NEXT_ENTRY..NEXT_ENTRY + 4]
                .copy_from_slice(&(if last { 0 } else { size as u32 }).to_le_bytes());
            e[WORKING_SET_PRIVATE..WORKING_SET_PRIVATE + 8].copy_from_slice(&private.to_le_bytes());
            e[CREATE_TIME..CREATE_TIME + 8].copy_from_slice(&created.to_le_bytes());
            e[IMAGE_NAME_LENGTH..IMAGE_NAME_LENGTH + 2]
                .copy_from_slice(&(name16.len() as u16).to_le_bytes());
            let ptr = if name.is_empty() {
                0
            } else {
                (base + start + ENTRY_SIZE) as u64
            };
            e[IMAGE_NAME_BUFFER..IMAGE_NAME_BUFFER + 8].copy_from_slice(&ptr.to_le_bytes());
            e[PROCESS_ID..PROCESS_ID + 8].copy_from_slice(&u64::from(pid).to_le_bytes());
            e[PARENT_ID..PARENT_ID + 8].copy_from_slice(&u64::from(parent).to_le_bytes());
            e[SESSION_ID..SESSION_ID + 4].copy_from_slice(&session.to_le_bytes());
            e[ENTRY_SIZE..ENTRY_SIZE + name16.len()].copy_from_slice(&name16);
        }
        buf
    }

    #[test]
    fn reads_every_entry_at_its_offsets() {
        let base = 0x1000_0000;
        let buf = synthetic(
            &[
                (0, 0, 0, 0, "", 8192),
                (4, 0, 1, 0, "System", 20_480),
                (
                    1234,
                    900,
                    133_000_000_000_000_000,
                    1,
                    "chrome.exe",
                    300 << 20,
                ),
            ],
            base,
        );
        let procs = parse(&buf, base);
        assert_eq!(procs.len(), 3);
        assert_eq!(procs[0].name, "");
        assert_eq!(procs[1].name, "System");
        assert_eq!(
            procs[2],
            RawProcess {
                pid: 1234,
                parent: 900,
                created: 133_000_000_000_000_000,
                session: 1,
                name: "chrome.exe".into(),
                private_bytes: 300 << 20,
            }
        );
    }

    #[test]
    fn stops_at_entries_and_names_out_of_bounds() {
        let base = 0x2000;
        let mut buf = synthetic(
            &[(10, 1, 5, 1, "a.exe", 1), (11, 1, 6, 1, "b.exe", 2)],
            base,
        );
        // A name pointer outside the buffer reads as no name instead of faulting.
        buf[IMAGE_NAME_BUFFER..IMAGE_NAME_BUFFER + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        // Truncate the second entry: it's dropped, not read past the end.
        let first = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
        buf.truncate(first + ENTRY_SIZE - 1);
        let procs = parse(&buf, base);
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].pid, 10);
        assert_eq!(procs[0].name, "");
    }

    #[test]
    fn live_snapshot_finds_this_process() {
        let procs = read().expect("snapshot");
        let own = std::process::id();
        let me = procs.iter().find(|p| p.pid == own).expect("own pid listed");
        assert!(me.private_bytes > 0);
        // The creation-time offset matches what GetProcessTimes reports for this process.
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let (mut c, mut e, mut k, mut u) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        // SAFETY: the pseudo-handle needs no closing; the FILETIMEs are valid out-pointers.
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) }.unwrap();
        let ticks = (u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime);
        assert_eq!(me.created, ticks);
        assert!(me.name.to_ascii_lowercase().ends_with(".exe"));
        assert!(procs.len() > 10);
    }
}
