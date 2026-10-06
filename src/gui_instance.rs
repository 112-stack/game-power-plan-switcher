// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! GUI-only ownership and bounded SHOW handover. No engine or power operations.
//! Acquire before initializing Slint. Kernel mutex abandonment recovers crashes;
//! the dedicated pipe never carries settings, paths, executable text or commands.
use sha2::{Digest, Sha256};
use std::{
    ffi::c_void,
    marker::PhantomData,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
type RawHandle = *mut c_void;
const INVALID_HANDLE: RawHandle = -1isize as RawHandle;
const WAIT_OBJECT: u32 = 0;
const WAIT_ABANDONED: u32 = 0x80;
const WAIT_TIMEOUT: u32 = 258;
const ERROR_IO_PENDING: u32 = 997;
const ERROR_PIPE_CONNECTED: u32 = 535;
const SHOW: &[u8; 8] = b"NN6SHOW1";
const ACK: &[u8; 8] = b"NN6ACK01";
const REJECT: &[u8; 8] = b"NN6ERR01";
const RECEIVED: &[u8; 8] = b"NN6DONE1";
#[repr(C)]
struct SecurityAttributes {
    size: u32,
    descriptor: *mut c_void,
    inherit: i32,
}
#[repr(C)]
#[derive(Default)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: RawHandle,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(
        attributes: *const SecurityAttributes,
        owner: i32,
        name: *const u16,
    ) -> RawHandle;
    fn ReleaseMutex(handle: RawHandle) -> i32;
    fn CloseHandle(handle: RawHandle) -> i32;
    fn GetLastError() -> u32;
    fn GetCurrentProcessId() -> u32;
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
    fn CreateEventW(
        attributes: *const SecurityAttributes,
        manual: i32,
        state: i32,
        name: *const u16,
    ) -> RawHandle;
    fn SetEvent(event: RawHandle) -> i32;
    fn WaitForSingleObject(handle: RawHandle, timeout: u32) -> u32;
    fn WaitForMultipleObjects(count: u32, handles: *const RawHandle, all: i32, timeout: u32)
    -> u32;
    fn CreateNamedPipeW(
        name: *const u16,
        access: u32,
        mode: u32,
        instances: u32,
        out_size: u32,
        in_size: u32,
        timeout: u32,
        attributes: *const SecurityAttributes,
    ) -> RawHandle;
    fn ConnectNamedPipe(pipe: RawHandle, overlapped: *mut Overlapped) -> i32;
    fn DisconnectNamedPipe(pipe: RawHandle) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        attributes: *const SecurityAttributes,
        disposition: u32,
        flags: u32,
        template: RawHandle,
    ) -> RawHandle;
    fn ReadFile(
        file: RawHandle,
        buffer: *mut c_void,
        size: u32,
        count: *mut u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn WriteFile(
        file: RawHandle,
        buffer: *const c_void,
        size: u32,
        count: *mut u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn GetOverlappedResult(
        file: RawHandle,
        overlapped: *mut Overlapped,
        count: *mut u32,
        wait: i32,
    ) -> i32;
    fn CancelIoEx(file: RawHandle, overlapped: *mut Overlapped) -> i32;
    fn GetNamedPipeServerProcessId(pipe: RawHandle, pid: *mut u32) -> i32;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl: *const u16,
        revision: u32,
        descriptor: *mut *mut c_void,
        size: *mut u32,
    ) -> i32;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn AllowSetForegroundWindow(pid: u32) -> i32;
    fn CharLowerBuffW(text: *mut u16, length: u32) -> u32;
    fn MessageBoxW(owner: RawHandle, text: *const u16, caption: *const u16, flags: u32) -> i32;
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn windows_error(operation: &str) -> String {
    let code = unsafe { GetLastError() };
    format!(
        "{operation}: {} ({code})",
        std::io::Error::from_raw_os_error(code as i32)
    )
}
struct Handle(RawHandle);
// Kernel handle values can cross threads; the mutex *ownership* guard below is
// separately !Send so ReleaseMutex always runs on the acquiring GUI thread.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Handle {
    fn checked(raw: RawHandle, operation: &str) -> Result<Self, String> {
        if raw.is_null() || raw == INVALID_HANDLE {
            Err(windows_error(operation))
        } else {
            Ok(Self(raw))
        }
    }
    fn event() -> Result<Self, String> {
        Self::checked(
            unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) },
            "Create GUI handover event",
        )
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Security(*mut c_void);
impl Security {
    fn user(sid: &str) -> Result<Self, String> {
        if !sid.starts_with("S-1-") || !sid[4..].bytes().all(|c| c.is_ascii_digit() || c == b'-') {
            return Err("Windows returned an invalid user SID".into());
        }
        let mut descriptor = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide(&format!("D:P(A;;GA;;;{sid})")).as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(windows_error("Create user-only GUI security descriptor"));
        }
        Ok(Self(descriptor))
    }
    fn attributes(&self) -> SecurityAttributes {
        SecurityAttributes {
            size: std::mem::size_of::<SecurityAttributes>() as u32,
            descriptor: self.0,
            inherit: 0,
        }
    }
}
impl Drop for Security {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
struct MutexOwnership {
    handle: Handle,
    _same_thread: PhantomData<Rc<()>>,
}
impl Drop for MutexOwnership {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle.0);
        }
    }
}

#[derive(Clone)]
struct Scope {
    key: String,
    sid: String,
    session: u32,
}
impl Scope {
    fn current(dry_run: bool) -> Result<Self, String> {
        let sid = crate::win::sid()?;
        let mut session = 0;
        if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
            return Err(windows_error("Read Windows session"));
        }
        let test_path = std::env::var_os("NN6_TEST_DATA_DIR").map(PathBuf::from);
        let marker = if let Some(path) = test_path {
            if path.as_os_str().is_empty() {
                return Err("NN6_TEST_DATA_DIR must not be empty".into());
            }
            let canonical = crate::runtime_scope::normalized_path(&path)?;
            let mut units: Vec<u16> = canonical.as_os_str().encode_wide().collect();
            // Canonicalize adds the extended-length prefix on Windows. Remove
            // it so a not-yet-created test directory has the same scope later.
            let unc: Vec<_> = r"\\?\UNC\".encode_utf16().collect();
            let extended: Vec<_> = r"\\?\".encode_utf16().collect();
            if units.starts_with(&unc) {
                units.splice(..unc.len(), [92, 92]);
            } else if units.starts_with(&extended) {
                units.drain(..extended.len());
            }
            for unit in &mut units {
                if *unit == 47 {
                    *unit = 92;
                }
            }
            unsafe {
                CharLowerBuffW(units.as_mut_ptr(), units.len() as u32);
            }
            units
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<u8>>()
        } else {
            b"production".to_vec()
        };
        Ok(Self::from_parts(sid, session, dry_run, &marker))
    }
    fn from_parts(sid: String, session: u32, dry_run: bool, marker: &[u8]) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"NN6.GUI.v1\0");
        digest.update((sid.len() as u64).to_le_bytes());
        digest.update(sid.as_bytes());
        digest.update(session.to_le_bytes());
        digest.update([u8::from(dry_run)]);
        digest.update((marker.len() as u64).to_le_bytes());
        digest.update(marker);
        Self {
            key: hex::encode(digest.finalize()),
            sid,
            session,
        }
    }
    fn mutex_name(&self) -> Vec<u16> {
        wide(&format!(r"Global\NN6.PowerPlan.GUI.v1.{}", self.key))
    }
    fn pipe_name(&self) -> Vec<u16> {
        wide(&format!(r"\\.\pipe\NN6.PowerPlan.GUI.v1.{}", self.key))
    }
}
type ShowHandler = Arc<dyn Fn() -> Result<(), String> + Send + Sync + 'static>;
#[derive(Default)]
struct Dispatch {
    handler: Option<ShowHandler>,
    pending: bool,
    requests: usize,
}
impl Dispatch {
    fn request(state: &Mutex<Self>) -> Result<(), String> {
        let handler = {
            let mut state = state.lock().map_err(|_| "GUI handover state poisoned")?;
            state.requests = state.requests.saturating_add(1);
            if state.handler.is_none() {
                state.pending = true;
                return Ok(());
            }
            state.handler.clone().unwrap()
        };
        handler()
    }
}
pub enum Launch {
    Primary(GuiInstance),
    Forwarded,
}
pub struct GuiInstance {
    _ownership: MutexOwnership,
    recovered_abandoned: bool,
    stop: Arc<Handle>,
    dispatch: Arc<Mutex<Dispatch>>,
    listener: Option<thread::JoinHandle<()>>,
}
impl GuiInstance {
    /// Handler must only queue SHOW on Slint's event loop, never block or call
    /// GUI objects directly. Early duplicate requests coalesce before install.
    pub fn set_show_handler(
        &self,
        handler: impl Fn() -> Result<(), String> + Send + Sync + 'static,
    ) -> Result<(), String> {
        let handler: ShowHandler = Arc::new(handler);
        let pending = {
            let mut state = self
                .dispatch
                .lock()
                .map_err(|_| "GUI handover state poisoned")?;
            state.handler = Some(handler.clone());
            std::mem::take(&mut state.pending)
        };
        if pending {
            handler()?;
        }
        Ok(())
    }
}
impl Drop for GuiInstance {
    fn drop(&mut self) {
        unsafe {
            SetEvent(self.stop.0);
        }
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
        // Listener's pipe is now closed. Only then release GUI mutex ownership.
    }
}

fn finish_io(
    handle: &Handle,
    overlapped: &mut Overlapped,
    stop: Option<&Handle>,
    timeout: u32,
) -> Result<u32, String> {
    let handles = [overlapped.event, stop.map_or(std::ptr::null_mut(), |s| s.0)];
    let waited = unsafe {
        WaitForMultipleObjects(
            if stop.is_some() { 2 } else { 1 },
            handles.as_ptr(),
            0,
            timeout,
        )
    };
    let mut count = 0;
    if waited != WAIT_OBJECT {
        unsafe {
            CancelIoEx(handle.0, overlapped);
            // The OVERLAPPED and its buffer must survive cancellation completion.
            GetOverlappedResult(handle.0, overlapped, &mut count, 1);
        }
        return Err(if waited == WAIT_TIMEOUT {
            "GUI handover I/O timed out"
        } else {
            "GUI handover stopped"
        }
        .into());
    }
    if unsafe { GetOverlappedResult(handle.0, overlapped, &mut count, 0) } == 0 {
        return Err(windows_error("Complete GUI handover I/O"));
    }
    Ok(count)
}
fn transfer(
    handle: &Handle,
    bytes: &mut [u8; 8],
    writing: bool,
    stop: Option<&Handle>,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut offset = 0;
    while offset < bytes.len() {
        let event = Handle::event()?;
        let mut overlapped = Overlapped {
            event: event.0,
            ..Default::default()
        };
        let mut count = 0;
        let ok = unsafe {
            if writing {
                WriteFile(
                    handle.0,
                    bytes[offset..].as_ptr().cast(),
                    (8 - offset) as u32,
                    &mut count,
                    &mut overlapped,
                )
            } else {
                ReadFile(
                    handle.0,
                    bytes[offset..].as_mut_ptr().cast(),
                    (8 - offset) as u32,
                    &mut count,
                    &mut overlapped,
                )
            }
        };
        if ok == 0 {
            if unsafe { GetLastError() } != ERROR_IO_PENDING {
                return Err(windows_error("Transfer GUI handover"));
            }
            let remaining = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(500) as u32;
            count = finish_io(handle, &mut overlapped, stop, remaining)?;
        }
        if count == 0 {
            return Err("GUI handover connection closed".into());
        }
        offset += count as usize;
    }
    Ok(())
}
fn listener(pipe: Handle, stop: Arc<Handle>, dispatch: Arc<Mutex<Dispatch>>) {
    while unsafe { WaitForSingleObject(stop.0, 0) } == WAIT_TIMEOUT {
        let Ok(event) = Handle::event() else { break };
        let mut overlapped = Overlapped {
            event: event.0,
            ..Default::default()
        };
        let connected = if unsafe { ConnectNamedPipe(pipe.0, &mut overlapped) } != 0 {
            true
        } else {
            match unsafe { GetLastError() } {
                ERROR_PIPE_CONNECTED => true,
                ERROR_IO_PENDING => {
                    finish_io(&pipe, &mut overlapped, Some(&stop), u32::MAX).is_ok()
                }
                _ => false,
            }
        };
        if connected {
            let mut request = [0; 8];
            if transfer(&pipe, &mut request, false, Some(&stop)).is_ok() {
                let mut answer = if &request == SHOW && Dispatch::request(&dispatch).is_ok() {
                    *ACK
                } else {
                    *REJECT
                };
                if transfer(&pipe, &mut answer, true, Some(&stop)).is_ok() {
                    // DisconnectNamedPipe discards unread output. Wait for a
                    // bounded client receipt rather than losing a buffered ACK
                    // or calling the potentially unbounded FlushFileBuffers.
                    let mut received = [0; 8];
                    let _ = transfer(&pipe, &mut received, false, Some(&stop));
                }
            }
        }
        unsafe {
            DisconnectNamedPipe(pipe.0);
        }
    }
}
fn primary(
    handle: Handle,
    scope: &Scope,
    security: &Security,
    recovered_abandoned: bool,
) -> Result<Launch, String> {
    let ownership = MutexOwnership {
        handle,
        _same_thread: PhantomData,
    };
    // FIRST_PIPE_INSTANCE prevents silently attaching to a competing server.
    let pipe = Handle::checked(
        unsafe {
            CreateNamedPipeW(
                scope.pipe_name().as_ptr(),
                3 | 0x40000000 | 0x00080000,
                8,
                1,
                64,
                64,
                500,
                &security.attributes(),
            )
        },
        "Create private GUI handover pipe",
    )?;
    let stop = Arc::new(Handle::event()?);
    let dispatch = Arc::new(Mutex::new(Dispatch::default()));
    let worker_stop = stop.clone();
    let worker_dispatch = dispatch.clone();
    let listener = thread::Builder::new()
        .name("nn6-gui-handover".into())
        .spawn(move || listener(pipe, worker_stop, worker_dispatch))
        .map_err(|e| e.to_string())?;
    Ok(Launch::Primary(GuiInstance {
        _ownership: ownership,
        recovered_abandoned,
        stop,
        dispatch,
        listener: Some(listener),
    }))
}
fn forward(scope: &Scope) -> Result<(), String> {
    // Identification-only SQOS prevents a pipe server impersonating this client.
    let pipe = Handle::checked(
        unsafe {
            CreateFileW(
                scope.pipe_name().as_ptr(),
                0xc0000000,
                0,
                std::ptr::null(),
                3,
                0x40000000 | 0x00100000 | 0x00010000,
                std::ptr::null_mut(),
            )
        },
        "Open existing GUI handover pipe",
    )?;
    let mut pid = 0;
    let mut session = u32::MAX;
    if unsafe { GetNamedPipeServerProcessId(pipe.0, &mut pid) } == 0
        || pid == 0
        || unsafe { ProcessIdToSessionId(pid, &mut session) } == 0
        || session != scope.session
    {
        return Err("GUI handover server is not in this Windows session".into());
    }
    // Best effort: Windows may deny focus transfer for background launches.
    // The existing GUI still restores itself and can flash its taskbar button.
    unsafe {
        AllowSetForegroundWindow(pid);
    }
    let mut request = *SHOW;
    transfer(&pipe, &mut request, true, None)?;
    let mut answer = [0; 8];
    transfer(&pipe, &mut answer, false, None)?;
    let mut received = *RECEIVED;
    let _ = transfer(&pipe, &mut received, true, None);
    if &answer == ACK {
        Ok(())
    } else {
        Err("Existing Game Power Plan Switcher could not queue SHOW; it may be closing".into())
    }
}
fn acquire_scope(scope: Scope) -> Result<Launch, String> {
    let security = Security::user(&scope.sid)?;
    let handle = Handle::checked(
        unsafe { CreateMutexW(&security.attributes(), 0, scope.mutex_name().as_ptr()) },
        "Open GUI singleton mutex",
    )?;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let ownership = unsafe { WaitForSingleObject(handle.0, 0) };
        match ownership {
            WAIT_OBJECT | WAIT_ABANDONED => {
                return primary(handle, &scope, &security, ownership == WAIT_ABANDONED);
            }
            WAIT_TIMEOUT => {}
            _ => return Err(windows_error("Check GUI singleton ownership")),
        }
        let last = match forward(&scope) {
            Ok(()) => return Ok(Launch::Forwarded),
            Err(error) => error,
        };
        if Instant::now() >= deadline {
            return Err(format!(
                "Game Power Plan Switcher is already running but did not accept a SHOW request within 8 seconds. No second window was started. {last}"
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
}
pub fn acquire(dry_run: bool) -> Result<Launch, String> {
    acquire_scope(Scope::current(dry_run)?)
}

/// Only GUI startup errors use this small native dialog; no Slint initialization.
pub fn show_startup_error(message: &str) {
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(message).as_ptr(),
            wide("Game Power Plan Switcher").as_ptr(),
            0x10,
        );
    }
}

/// Explicit isolated diagnostic. Does not create a GUI, daemon or guardian,
/// register hotkeys/startup, read game processes, or change any power plan.
pub fn probe(report: &Path, hold_ms: u64) -> Result<(), String> {
    let test = std::env::var_os("NN6_TEST_DATA_DIR")
        .ok_or("GUI singleton probe requires an isolated NN6_TEST_DATA_DIR")?;
    if test.is_empty() || !report.is_absolute() {
        return Err("Use a nonempty test directory and absolute report path".into());
    }
    if !(100..=60000).contains(&hold_ms) {
        return Err("Probe hold_ms must be between 100 and 60000".into());
    }
    std::fs::create_dir_all(&test).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let pid = std::process::id();
    let scope = Scope::current(false)?;
    let key = scope.key.clone();
    match acquire_scope(scope)? {
        Launch::Forwarded => crate::model::atomic_json(
            report,
            &serde_json::json!({"pid":pid,"scope_key":key,"role":"forwarded","elapsed_ms":started.elapsed().as_millis(),"passed":true}),
        ),
        Launch::Primary(instance) => {
            let recovered = instance.recovered_abandoned;
            let received = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let count = received.clone();
            instance.set_show_handler(move || {
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            })?;
            crate::model::atomic_json(
                report,
                &serde_json::json!({"pid":pid,"scope_key":key,"role":"primary","phase":"listening","recovered_abandoned":recovered,"passed":true}),
            )?;
            thread::sleep(Duration::from_millis(hold_ms));
            let requests = instance
                .dispatch
                .lock()
                .map_err(|_| "GUI handover state poisoned")?
                .requests;
            drop(instance);
            crate::model::atomic_json(
                report,
                &serde_json::json!({"pid":pid,"scope_key":key,"role":"primary","phase":"closed","recovered_abandoned":recovered,"show_requests":requests,"show_dispatches":received.load(std::sync::atomic::Ordering::Relaxed),"elapsed_ms":started.elapsed().as_millis(),"passed":true}),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespace_separates_user_session_dryrun_and_fixture() {
        let scope =
            |sid, session, dry, fixture: &[u8]| Scope::from_parts(sid, session, dry, fixture).key;
        let base = scope("S-1-5-21-1".into(), 1, false, b"production");
        for other in [
            scope("S-1-5-21-2".into(), 1, false, b"production"),
            scope("S-1-5-21-1".into(), 2, false, b"production"),
            scope("S-1-5-21-1".into(), 1, true, b"production"),
            scope("S-1-5-21-1".into(), 1, false, b"fixture"),
        ] {
            assert_ne!(base, other);
        }
    }
    #[test]
    fn early_show_requests_coalesce_without_losing_install_race() {
        let state = Mutex::new(Dispatch::default());
        for _ in 0..30 {
            Dispatch::request(&state).unwrap();
        }
        assert!(state.lock().unwrap().pending);
        assert!(state.lock().unwrap().handler.is_none());
    }
    #[test]
    fn private_kernel_owner_handover_and_clean_reacquire() {
        let mut scope = Scope::current(true).unwrap();
        scope.key = format!("test-{}-{}", std::process::id(), crate::model::now_ms());
        let Launch::Primary(primary) = acquire_scope(scope.clone()).unwrap() else {
            panic!("fresh test scope")
        };
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = requests.clone();
        primary
            .set_show_handler(move || {
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            })
            .unwrap();
        let workers: Vec<_> = (0..30)
            .map(|_| {
                let scope = scope.clone();
                thread::spawn(move || matches!(acquire_scope(scope).unwrap(), Launch::Forwarded))
            })
            .collect();
        for worker in workers {
            assert!(worker.join().unwrap());
        }
        assert_eq!(requests.load(std::sync::atomic::Ordering::Relaxed), 30);
        drop(primary);
        assert!(matches!(acquire_scope(scope).unwrap(), Launch::Primary(_)));
    }
}
