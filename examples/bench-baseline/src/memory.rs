use super::report::Memory;

pub fn sample() -> Memory {
    #[cfg(windows)]
    {
        windows_sample()
    }
    #[cfg(not(windows))]
    {
        Memory {
            measured: false,
            method: "not measured on this operating system",
            working_set_bytes: None,
            peak_working_set_bytes: None,
        }
    }
}

#[cfg(windows)]
fn windows_sample() -> Memory {
    #[repr(C)]
    struct Counters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    extern "system" {
        fn GetCurrentProcess() -> *mut core::ffi::c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut core::ffi::c_void,
            counters: *mut Counters,
            cb: u32,
        ) -> i32;
    }

    let mut counters = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        page_fault_count: 0,
        peak_working_set_size: 0,
        working_set_size: 0,
        quota_peak_paged_pool_usage: 0,
        quota_paged_pool_usage: 0,
        quota_peak_non_paged_pool_usage: 0,
        quota_non_paged_pool_usage: 0,
        pagefile_usage: 0,
        peak_pagefile_usage: 0,
    };
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    if ok == 0 {
        return Memory {
            measured: false,
            method: "K32GetProcessMemoryInfo failed",
            working_set_bytes: None,
            peak_working_set_bytes: None,
        };
    }
    Memory {
        measured: true,
        method: "K32GetProcessMemoryInfo working set",
        working_set_bytes: Some(counters.working_set_size as u64),
        peak_working_set_bytes: Some(counters.peak_working_set_size as u64),
    }
}
