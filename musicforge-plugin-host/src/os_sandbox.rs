//! 插件 OS 级沙箱（B15 后续，可行子集）。
//!
//! 桌面 Tauri 应用无法像 MSIX 那样把插件注册为 AppContainer（文件/网络强隔离），
//! 故本模块只做**进程生命周期容器**这一可落地、零风险的部分：
//! 把插件子进程纳入 Windows **Job Object** 并设 `KillOnJobClose`——
//! 宿主退出/崩溃时作业内进程随之一并终止，杜绝孤儿插件进程或「宿主已死仍运行」的逃逸。
//!
//! 用 raw Win32 FFI（`repr(C)` 与 `winnt.h` 字节一致），不引入 windows-sys 等重型依赖，
//! 避免其 feature 门控在不同环境下解析不一致。失败仅 `tracing::warn` 后放行
//! （优雅降级：插件仍按协议运行），绝不阻断 spawn。非 Windows 平台为空实现。

#[cfg(windows)]
#[allow(non_camel_case_types, non_upper_case_globals, clippy::upper_case_acronyms)]
mod imp {
    use std::os::raw::c_void;

    type HANDLE = *mut c_void;
    type BOOL = i32;
    type DWORD = u32;
    type SIZE_T = usize;
    type ULONG_PTR = usize;

    const FALSE: BOOL = 0;
    /// 作业最后一个句柄关闭时，作业内进程全部终止（宿主流产/崩溃均生效）。
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: DWORD = 0x0000_2000;
    const JobObjectBasicLimitInformation: u32 = 2;
    /// 需要 `PROCESS_SET_QUOTA | PROCESS_TERMINATE` 才能 AssignProcessToJobObject。
    const PROCESS_ALL_ACCESS: DWORD = 0x001F_0FFF;

    /// 与 `winnt.h` 字节一致的精简版基本限额结构（仅需 KillOnJobClose 一项）。
    #[repr(C)]
    struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: DWORD,
        minimum_working_set_size: SIZE_T,
        maximum_working_set_size: SIZE_T,
        active_process_limit: DWORD,
        affinity: ULONG_PTR,
        priority_class: DWORD,
        scheduling_class: DWORD,
    }

    extern "system" {
        fn CreateJobObjectW(job_attributes: *const c_void, name: *const u16) -> HANDLE;
        fn AssignProcessToJobObject(job: HANDLE, process: HANDLE) -> BOOL;
        fn SetInformationJobObject(
            job: HANDLE,
            info_class: u32,
            info: *const c_void,
            info_length: DWORD,
        ) -> BOOL;
        fn OpenProcess(desired_access: DWORD, inherit_handle: BOOL, process_id: DWORD) -> HANDLE;
        fn CloseHandle(object: HANDLE) -> BOOL;
    }

    /// 将已 spawn 的子进程 `pid` 纳入作业（kill-on-job-close）。
    pub fn attach(pid: u32) {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                tracing::warn!("沙箱：CreateJobObjectW 失败，跳过 OS 沙箱");
                return;
            }
            let info = JOBOBJECT_BASIC_LIMIT_INFORMATION {
                per_process_user_time_limit: 0,
                per_job_user_time_limit: 0,
                limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                minimum_working_set_size: 0,
                maximum_working_set_size: 0,
                active_process_limit: 0,
                affinity: 0,
                priority_class: 0,
                scheduling_class: 0,
            };
            let ok = SetInformationJobObject(
                job,
                JobObjectBasicLimitInformation,
                &info as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_BASIC_LIMIT_INFORMATION>() as DWORD,
            ) != 0;
            if ok {
                let proc = OpenProcess(PROCESS_ALL_ACCESS, FALSE, pid);
                if !proc.is_null() {
                    // 把子进程放入作业：宿主退出/崩溃时随作业一并终止（杜绝孤儿/逃逸）
                    if AssignProcessToJobObject(job, proc) == 0 {
                        // 子进程可能已在其他作业中（如被父进程继承的作业）——放行不阻断
                        tracing::warn!("沙箱：AssignProcessToJobObject 失败（子进程或已在其他作业中），未启用 OS 沙箱，协议仍可运行");
                    }
                    CloseHandle(proc);
                } else {
                    tracing::warn!("沙箱：OpenProcess 失败（pid={pid}），跳过 OS 沙箱");
                }
            } else {
                tracing::warn!("沙箱：SetInformationJobObject 失败，跳过 OS 沙箱");
            }
            // **关键点**：不可关闭作业句柄。本作业唯一句柄即由宿主持有——
            // 若在此 CloseHandle，则「最后一个句柄关闭」立即触发 kill-on-job-close，
            // 插件刚被纳入就立刻被杀。保留句柄直到宿主进程退出（或崩溃），
            // 届时 OS 回收该句柄 → 作业最后一个句柄关闭 → 插件随宿主一并终止
            // （这正是「宿主死了插件也死、杜绝孤儿进程」的语义）。句柄随宿主进程退出而释放，无需显式关闭。
        }
    }
}

#[cfg(not(windows))]
mod imp {
    /// 非 Windows：空实现（作业对象为 Windows 专有）。
    pub fn attach(_pid: u32) {}
}

pub use imp::attach;

#[cfg(all(test, windows))]
mod tests {
    use super::attach;
    use std::os::raw::c_void;
    use std::process::Command;

    extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> *mut c_void;
        fn IsProcessInJob(process: *mut c_void, job: *mut c_void, result: *mut i32) -> i32;
        fn CloseHandle(object: *mut c_void) -> i32;
    }

    const PROCESS_ALL_ACCESS: u32 = 0x001F_0FFF;

    /// 回归：attach 把真实子进程纳入 Job Object（kill-on-job-close），
    /// 且子进程在 attach 后**仍存活**（句柄保留直至宿主退出，而非立即被杀）。
    #[test]
    fn attach_puts_child_in_job_and_keeps_it_alive() {
        let mut child = Command::new("cmd")
            .args(["/c", "timeout", "30"])
            .spawn()
            .expect("应能启动子进程用于沙箱测试");
        let pid = child.id();
        attach(pid);
        unsafe {
            let h = OpenProcess(PROCESS_ALL_ACCESS, 0, pid);
            assert!(!h.is_null(), "OpenProcess 应成功（子进程应仍在运行）");
            let mut in_job: i32 = 0;
            // job=NULL → 查询「是否处于任意作业」
            let ok = IsProcessInJob(h, std::ptr::null_mut(), &mut in_job);
            CloseHandle(h);
            assert!(
                ok != 0 && in_job != 0,
                "子进程应已被纳入 Job Object（kill-on-job-close）"
            );
        }
        // attach 不应立即杀掉插件：子进程仍存活
        assert!(
            child.try_wait().expect("try_wait 应成功").is_none(),
            "attach 后子进程应仍存活（句柄保留至宿主退出）"
        );
        let _ = child.kill();
    }
}
