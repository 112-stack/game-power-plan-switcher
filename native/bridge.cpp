// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
// Narrow C ABI. Rust owns application state; C++23 owns COM and Win32 calls.
#define WIN32_LEAN_AND_MEAN
#define _WIN32_WINNT 0x0A00
#include <windows.h>
#include <algorithm>
#include <atomic>
#include <commdlg.h>
#include <cmath>
#include <cstring>
#include <dwmapi.h>
#include <pdh.h>
#include <pdhmsg.h>
#include <mutex>
#include <powersetting.h>
#include <powrprof.h>
#include <sddl.h>
#include <shellapi.h>
#include <string>
#include <tlhelp32.h>
#include <vector>
#include <wbemidl.h>
using EventFn = void (*)(unsigned, unsigned, const wchar_t *,
                         unsigned long long);
static EventFn event_fn = nullptr;
static std::atomic<DWORD> event_thread = 0;
static std::atomic<bool> stopping = false;
static std::atomic<unsigned long long> event_generation = 0;
static void text(const std::wstring &s, wchar_t *out, unsigned n) {
  if (n) {
    wcsncpy(out, s.c_str(), n - 1);
    out[n - 1] = 0;
  }
}
static std::wstring guid_text(const GUID &g) {
  wchar_t b[40] = {};
  StringFromGUID2(g, b, 40);
  return std::wstring(b).substr(1, 36);
}
static unsigned long long stamp() { return event_generation.load(); }
static WNDPROC previous_window_proc = nullptr;
static LRESULT CALLBACK app_window_proc(HWND h, UINT msg, WPARAM w, LPARAM l) {
  if (msg == WM_GETMINMAXINFO) {
    auto result = CallWindowProcW(previous_window_proc, h, msg, w, l);
    MONITORINFO mi{sizeof(mi)};
    if (GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mi)) {
      auto m = (MINMAXINFO *)l;
      m->ptMaxPosition = {mi.rcWork.left - mi.rcMonitor.left,
                          mi.rcWork.top - mi.rcMonitor.top};
      m->ptMaxSize = {mi.rcWork.right - mi.rcWork.left,
                      mi.rcWork.bottom - mi.rcWork.top};
    }
    return result;
  }
  return CallWindowProcW(previous_window_proc, h, msg, w, l);
}
struct ProcessWait {
  HANDLE process = nullptr;
  HANDLE registration = nullptr;
  unsigned pid = 0;
  unsigned long long generation = stamp();
};
// Bounded shared-memory telemetry ring. A per-user mutex makes ownership and
// copying explicit; Protocol Buffers decoding is deliberately not called
// zero-copy.
struct TelemetrySlot {
  unsigned long long sequence;
  unsigned length;
  unsigned char data[65536];
};
struct TelemetryMemory {
  unsigned long long next;
  TelemetrySlot slots[8];
};
struct TelemetryMap {
  HANDLE mapping = nullptr;
  HANDLE mutex = nullptr;
  TelemetryMemory *view = nullptr;
  bool writer = false;
};
static VOID CALLBACK process_exited(PVOID context, BOOLEAN) {
  auto w = (ProcessWait *)context;
  if (event_fn)
    event_fn(2, w->pid, L"", w->generation);
}
extern "C" {
void nn6_ring_close(void *context) {
  auto m = (TelemetryMap *)context;
  if (m) {
    if (m->view)
      UnmapViewOfFile(m->view);
    if (m->mapping)
      CloseHandle(m->mapping);
    if (m->mutex)
      CloseHandle(m->mutex);
    delete m;
  }
}
void *nn6_ring_open(const wchar_t *name, bool writer) {
  wchar_t sid[256] = {};
  HANDLE token = nullptr;
  if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token))
    return nullptr;
  DWORD bytes = 0;
  GetTokenInformation(token, TokenUser, nullptr, 0, &bytes);
  std::vector<BYTE> buffer(bytes);
  if (!GetTokenInformation(token, TokenUser, buffer.data(), bytes, &bytes)) {
    CloseHandle(token);
    return nullptr;
  }
  LPWSTR sidString = nullptr;
  ConvertSidToStringSidW(((TOKEN_USER *)buffer.data())->User.Sid, &sidString);
  CloseHandle(token);
  if (!sidString)
    return nullptr;
  text(sidString, sid, 256);
  LocalFree(sidString);
  std::wstring d = L"D:P(A;;GA;;;" + std::wstring(sid) + L")";
  PSECURITY_DESCRIPTOR sd = nullptr;
  if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
          d.c_str(), SDDL_REVISION_1, &sd, nullptr))
    return nullptr;
  SECURITY_ATTRIBUTES sa{sizeof(sa), sd, FALSE};
  auto m = new TelemetryMap();
  m->writer = writer;
  std::wstring mutexName = std::wstring(name) + L"-mutex";
  m->mutex = writer ? CreateMutexW(&sa, FALSE, mutexName.c_str())
                    : OpenMutexW(SYNCHRONIZE | MUTEX_MODIFY_STATE, FALSE,
                                 mutexName.c_str());
  m->mapping =
      writer ? CreateFileMappingW(INVALID_HANDLE_VALUE, &sa, PAGE_READWRITE, 0,
                                  sizeof(TelemetryMemory), name)
             : OpenFileMappingW(FILE_MAP_READ, FALSE, name);
  LocalFree(sd);
  if (m->mapping)
    m->view = (TelemetryMemory *)MapViewOfFile(
        m->mapping, writer ? FILE_MAP_ALL_ACCESS : FILE_MAP_READ, 0, 0,
        sizeof(TelemetryMemory));
  if (!m->view || !m->mutex) {
    nn6_ring_close(m);
    return nullptr;
  }
  return m;
}
unsigned nn6_ring_write(void *context, const unsigned char *data, unsigned size,
                        unsigned long long *sequence) {
  auto m = (TelemetryMap *)context;
  if (!m || !m->writer || size > 65536)
    return ERROR_INVALID_PARAMETER;
  auto wait = WaitForSingleObject(m->mutex, 1000);
  if (wait != WAIT_OBJECT_0 && wait != WAIT_ABANDONED)
    return ERROR_TIMEOUT;
  auto seq = ++m->view->next;
  auto &slot = m->view->slots[seq % 8];
  slot.sequence = 0;
  slot.length = size;
  memcpy(slot.data, data, size);
  slot.sequence = seq;
  *sequence = seq;
  ReleaseMutex(m->mutex);
  return 0;
}
unsigned nn6_ring_read(void *context, unsigned long long sequence,
                       unsigned char *out, unsigned cap, unsigned *length) {
  auto m = (TelemetryMap *)context;
  if (!m)
    return ERROR_INVALID_PARAMETER;
  auto wait = WaitForSingleObject(m->mutex, 1000);
  if (wait != WAIT_OBJECT_0 && wait != WAIT_ABANDONED)
    return ERROR_TIMEOUT;
  auto next = m->view->next;
  if (sequence > next || next - sequence >= 8)
    sequence = next;
  auto &slot = m->view->slots[sequence % 8];
  unsigned e = 0;
  if (slot.sequence != sequence || slot.length > cap || slot.length > 65536)
    e = ERROR_INVALID_DATA;
  else {
    *length = slot.length;
    memcpy(out, slot.data, slot.length);
  }
  ReleaseMutex(m->mutex);
  return e;
}
void *nn6_watch_exit(unsigned pid) {
  auto w = new ProcessWait();
  w->pid = pid;
  w->process = OpenProcess(SYNCHRONIZE, FALSE, pid);
  if (!w->process) {
    delete w;
    return nullptr;
  }
  if (!RegisterWaitForSingleObject(&w->registration, w->process, process_exited,
                                   w, INFINITE, WT_EXECUTEONLYONCE)) {
    CloseHandle(w->process);
    delete w;
    return nullptr;
  }
  return w;
}
void nn6_unwatch_exit(void *context) {
  auto w = (ProcessWait *)context;
  if (w) {
    if (w->registration)
      UnregisterWaitEx(w->registration, INVALID_HANDLE_VALUE);
    CloseHandle(w->process);
    delete w;
  }
}
unsigned nn6_sid(wchar_t *out, unsigned cap) {
  HANDLE token = nullptr;
  if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token))
    return GetLastError();
  DWORD n = 0;
  GetTokenInformation(token, TokenUser, nullptr, 0, &n);
  std::vector<BYTE> data(n);
  if (!GetTokenInformation(token, TokenUser, data.data(), n, &n)) {
    auto e = GetLastError();
    CloseHandle(token);
    return e;
  }
  LPWSTR str = nullptr;
  BOOL ok = ConvertSidToStringSidW(
      reinterpret_cast<TOKEN_USER *>(data.data())->User.Sid, &str);
  if (ok) {
    text(str, out, cap);
    LocalFree(str);
  }
  CloseHandle(token);
  return ok ? 0 : GetLastError();
}
unsigned nn6_active(wchar_t *out, unsigned cap) {
  GUID *g = nullptr;
  auto e = PowerGetActiveScheme(nullptr, &g);
  if (!e && g) {
    text(guid_text(*g), out, cap);
    LocalFree(g);
  }
  return e;
}
unsigned nn6_set_plan(const wchar_t *str) {
  GUID g{};
  auto e = CLSIDFromString(str, &g);
  return e ? e : PowerSetActiveScheme(nullptr, &g);
}
unsigned nn6_plans(wchar_t *out, unsigned cap) {
  std::wstring result;
  for (DWORD i = 0;; ++i) {
    GUID g{};
    DWORD size = sizeof(g);
    auto e = PowerEnumerate(nullptr, nullptr, nullptr, ACCESS_SCHEME, i,
                            (BYTE *)&g, &size);
    if (e == ERROR_NO_MORE_ITEMS)
      break;
    if (e)
      return e;
    DWORD bytes = 0;
    PowerReadFriendlyName(nullptr, &g, nullptr, nullptr, nullptr, &bytes);
    std::vector<BYTE> b(bytes + 2, 0);
    e = PowerReadFriendlyName(nullptr, &g, nullptr, nullptr, b.data(), &bytes);
    if (e)
      return e;
    result +=
        guid_text(g) + L"\t" + reinterpret_cast<wchar_t *>(b.data()) + L"\n";
  }
  if (result.size() + 1 > cap)
    return ERROR_INSUFFICIENT_BUFFER;
  text(result, out, cap);
  return 0;
}
unsigned nn6_overlay(wchar_t *out, unsigned cap) {
  using F = DWORD(WINAPI *)(GUID *);
  auto f = (F)GetProcAddress(GetModuleHandleW(L"powrprof.dll"),
                             "PowerGetActualOverlayScheme");
  if (!f)
    return ERROR_NOT_SUPPORTED;
  GUID g{};
  auto e = f(&g);
  if (!e)
    text(guid_text(g), out, cap);
  return e;
}
unsigned nn6_set_overlay(const wchar_t *str) {
  using F = DWORD(WINAPI *)(GUID);
  auto f = (F)GetProcAddress(GetModuleHandleW(L"powrprof.dll"),
                             "PowerSetActiveOverlayScheme");
  if (!f)
    return ERROR_NOT_SUPPORTED;
  GUID g{};
  auto e = CLSIDFromString(str, &g);
  return e ? e : f(g);
}
unsigned nn6_processes(wchar_t *out, unsigned cap) {
  HANDLE h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
  if (h == INVALID_HANDLE_VALUE)
    return GetLastError();
  PROCESSENTRY32W p{};
  p.dwSize = sizeof(p);
  std::wstring result;
  if (Process32FirstW(h, &p)) {
    do {
      result += std::to_wstring(p.th32ProcessID) + L"\t" + p.szExeFile + L"\n";
    } while (Process32NextW(h, &p));
  }
  CloseHandle(h);
  if (result.size() + 1 > cap)
    return ERROR_INSUFFICIENT_BUFFER;
  text(result, out, cap);
  return 0;
}
struct Schedule {
  unsigned pid;
  unsigned priority;
  unsigned long long affinity;
  unsigned long long created;
};
unsigned nn6_schedule(unsigned pid, unsigned priority,
                      unsigned long long affinity, unsigned long long expected,
                      Schedule *before) {
  HANDLE h = OpenProcess(PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION,
                         FALSE, pid);
  if (!h)
    return GetLastError();
  DWORD_PTR pm = 0, sm = 0;
  FILETIME c, x, k, u;
  unsigned e = 0;
  if (!GetProcessAffinityMask(h, &pm, &sm) ||
      !GetProcessTimes(h, &c, &x, &k, &u)) {
    e = GetLastError();
    CloseHandle(h);
    return e;
  }
  *before = {pid, GetPriorityClass(h), (unsigned long long)pm,
             ((unsigned long long)c.dwHighDateTime << 32) | c.dwLowDateTime};
  if (expected && before->created != expected) {
    CloseHandle(h);
    return ERROR_INVALID_PARAMETER;
  }
  if (affinity && ((affinity & sm) != affinity)) {
    CloseHandle(h);
    return ERROR_INVALID_PARAMETER;
  }
  if (priority && !SetPriorityClass(h, priority))
    e = GetLastError();
  if (!e && affinity && !SetProcessAffinityMask(h, (DWORD_PTR)affinity)) {
    e = GetLastError();
    if (priority)
      SetPriorityClass(h, before->priority);
  }
  CloseHandle(h);
  return e;
}
unsigned nn6_restore_schedule(const Schedule *s) {
  HANDLE h = OpenProcess(PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION,
                         FALSE, s->pid);
  if (!h) {
    auto e = GetLastError();
    return e == ERROR_INVALID_PARAMETER ? 0 : e;
  }
  FILETIME c, x, k, u;
  unsigned e = 0;
  if (!GetProcessTimes(h, &c, &x, &k, &u))
    e = GetLastError();
  else if ((((unsigned long long)c.dwHighDateTime << 32) | c.dwLowDateTime) !=
           s->created) {
    CloseHandle(h);
    return 0;
  } else {
    if (!SetPriorityClass(h, s->priority))
      e = GetLastError();
    if (!SetProcessAffinityMask(h, (DWORD_PTR)s->affinity))
      e = GetLastError();
  }
  CloseHandle(h);
  return e;
}
struct Core {
  unsigned logical;
  unsigned core;
  unsigned group;
  unsigned efficiency;
  unsigned parked;
};
unsigned nn6_cores(Core *out, unsigned cap) {
  DWORD bytes = 0;
  GetSystemCpuSetInformation(nullptr, 0, &bytes, GetCurrentProcess(), 0);
  if (!bytes)
    return 0;
  std::vector<BYTE> data(bytes);
  if (!GetSystemCpuSetInformation((PSYSTEM_CPU_SET_INFORMATION)data.data(),
                                  bytes, &bytes, GetCurrentProcess(), 0))
    return 0;
  unsigned count = 0;
  for (DWORD pos = 0; pos + sizeof(DWORD) <= bytes;) {
    auto s = (PSYSTEM_CPU_SET_INFORMATION)(data.data() + pos);
    if (!s->Size || pos + s->Size > bytes)
      break;
    if (s->Type == CpuSetInformation && count < cap) {
      auto &c = s->CpuSet;
      out[count++] = {c.LogicalProcessorIndex, c.CoreIndex, c.Group,
                      c.EfficiencyClass, (unsigned)c.Parked};
    }
    pos += s->Size;
  }
  return count;
}
// PDH rate counters need two samples. Reset when the Overview becomes hidden so
// resuming cannot report an average over the entire hidden interval as "live".
struct CpuLoad {
  unsigned group;
  unsigned logical;
  double load;
};
static std::mutex cpu_mutex;
static PDH_HQUERY cpu_query = nullptr;
static PDH_HCOUNTER cpu_counter = nullptr;
static ULONGLONG cpu_collected_at = 0;
static void reset_cpu_query() {
  if (cpu_query)
    PdhCloseQuery(cpu_query);
  cpu_query = nullptr;
  cpu_counter = nullptr;
  cpu_collected_at = 0;
}
void nn6_cpu_reset() {
  std::lock_guard lock(cpu_mutex);
  reset_cpu_query();
}
unsigned nn6_cpu_load(CpuLoad *out, unsigned cap, unsigned *written,
                      unsigned long long *interval_ms) {
  std::lock_guard lock(cpu_mutex);
  *written = 0;
  *interval_ms = 0;
  auto fail = [](PDH_STATUS status) -> unsigned {
    reset_cpu_query();
    return (unsigned)status;
  };
  if (!cpu_query) {
    auto status = PdhOpenQueryW(nullptr, 0, &cpu_query);
    if (status != ERROR_SUCCESS)
      return fail(status);
    status = PdhAddEnglishCounterW(
        cpu_query, L"\\Processor Information(*)\\% Processor Time", 0,
        &cpu_counter);
    if (status != ERROR_SUCCESS)
      return fail(status);
  }
  auto status = PdhCollectQueryData(cpu_query);
  if (status != ERROR_SUCCESS)
    return fail(status);
  auto collected = GetTickCount64();
  if (!cpu_collected_at) {
    cpu_collected_at = collected;
    return ERROR_IO_PENDING; // Explicit warm-up, never a synthetic zero sample.
  }
  *interval_ms = collected - cpu_collected_at;
  cpu_collected_at = collected;
  DWORD bytes = 0, count = 0;
  status = PdhGetFormattedCounterArrayW(cpu_counter, PDH_FMT_DOUBLE, &bytes,
                                        &count, nullptr);
  if (status != (PDH_STATUS)PDH_MORE_DATA)
    return fail(status == ERROR_SUCCESS ? (PDH_STATUS)PDH_NO_DATA : status);
  std::vector<BYTE> data(bytes);
  auto items = (PPDH_FMT_COUNTERVALUE_ITEM_W)data.data();
  status = PdhGetFormattedCounterArrayW(cpu_counter, PDH_FMT_DOUBLE, &bytes,
                                        &count, items);
  if (status != ERROR_SUCCESS)
    return fail(status);
  for (DWORD i = 0; i < count; i++) {
    unsigned group = 0, index = 0;
    wchar_t trailing = 0;
    // Processor Information instances are "group,logical-index". Keep both
    // keys; processor 0 in group 1 is not processor 0 in group 0. Skip _Total.
    if (swscanf(items[i].szName, L"%u,%u%lc", &group, &index, &trailing) == 2 &&
        (items[i].FmtValue.CStatus == PDH_CSTATUS_VALID_DATA ||
         items[i].FmtValue.CStatus == PDH_CSTATUS_NEW_DATA) &&
        std::isfinite(items[i].FmtValue.doubleValue)) {
      if (*written == cap)
        return ERROR_MORE_DATA;
      out[(*written)++] = {
          group, index, std::clamp(items[i].FmtValue.doubleValue, 0.0, 100.0)};
    }
  }
  return *written ? ERROR_SUCCESS : (unsigned)PDH_NO_DATA;
}
unsigned nn6_pick_exes(void *owner, wchar_t *out, unsigned cap) {
  OPENFILENAMEW d{};
  d.lStructSize = sizeof(d);
  d.hwndOwner = (HWND)owner;
  d.lpstrFilter = L"Applications (*.exe)\0*.exe\0";
  d.lpstrFile = out;
  d.nMaxFile = cap;
  d.lpstrTitle = L"Add game executables";
  d.Flags =
      OFN_EXPLORER | OFN_ALLOWMULTISELECT | OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR;
  return GetOpenFileNameW(&d) ? 1 : 0;
}
unsigned nn6_icon(const wchar_t *path, unsigned char *rgba, unsigned side) {
  if (side == 0 || side > 128)
    return ERROR_INVALID_PARAMETER;
  HICON icon = nullptr;
  if (!ExtractIconExW(path, 0, &icon, nullptr, 1) || !icon)
    return ERROR_FILE_NOT_FOUND;
  HDC dc = CreateCompatibleDC(nullptr);
  BITMAPINFO bi{};
  bi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
  bi.bmiHeader.biWidth = side;
  bi.bmiHeader.biHeight = -(LONG)side;
  bi.bmiHeader.biPlanes = 1;
  bi.bmiHeader.biBitCount = 32;
  bi.bmiHeader.biCompression = BI_RGB;
  void *bits = nullptr;
  HBITMAP bm = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &bits, nullptr, 0);
  if (!bm) {
    DestroyIcon(icon);
    DeleteDC(dc);
    return ERROR_NOT_ENOUGH_MEMORY;
  }
  auto prev = SelectObject(dc, bm);
  memset(bits, 0, side * side * 4);
  DrawIconEx(dc, 0, 0, icon, side, side, 0, nullptr, DI_NORMAL);
  auto b = (BYTE *)bits;
  bool alpha = false;
  for (unsigned i = 0; i < side * side; i++)
    if (b[i * 4 + 3]) {
      alpha = true;
      break;
    }
  for (unsigned i = 0; i < side * side; i++) {
    rgba[i * 4] = b[i * 4 + 2];
    rgba[i * 4 + 1] = b[i * 4 + 1];
    rgba[i * 4 + 2] = b[i * 4];
    rgba[i * 4 + 3] = alpha ? b[i * 4 + 3] : 255;
  }
  SelectObject(dc, prev);
  DeleteObject(bm);
  DeleteDC(dc);
  DestroyIcon(icon);
  return 0;
}
unsigned nn6_open_url(const wchar_t *url) {
  return (INT_PTR)ShellExecuteW(nullptr, L"open", url, nullptr, nullptr,
                                SW_SHOWNORMAL) <= 32
             ? 1
             : 0;
}
unsigned nn6_clipboard(void *owner, const wchar_t *str) {
  if (!OpenClipboard((HWND)owner))
    return GetLastError();
  auto bytes = (wcslen(str) + 1) * sizeof(wchar_t);
  HGLOBAL h = GlobalAlloc(GMEM_MOVEABLE, bytes);
  if (!h) {
    CloseClipboard();
    return ERROR_NOT_ENOUGH_MEMORY;
  }
  auto p = GlobalLock(h);
  memcpy(p, str, bytes);
  GlobalUnlock(h);
  EmptyClipboard();
  if (!SetClipboardData(CF_UNICODETEXT, h)) {
    GlobalFree(h);
    CloseClipboard();
    return GetLastError();
  }
  CloseClipboard();
  return 0;
}
void nn6_window(void *hwnd, unsigned action) {
  if (action == 5) {
    ShowWindow((HWND)hwnd, SW_SHOW);
    SetForegroundWindow((HWND)hwnd);
  }
  if (action == 0)
    PostMessageW((HWND)hwnd, WM_CLOSE, 0, 0);
  if (action == 1)
    ShowWindow((HWND)hwnd, SW_MINIMIZE);
  if (action == 2)
    ShowWindow((HWND)hwnd, IsZoomed((HWND)hwnd) ? SW_RESTORE : SW_MAXIMIZE);
  if (action == 3) {
    ReleaseCapture();
    SendMessageW((HWND)hwnd, WM_NCLBUTTONDOWN, HTCAPTION, 0);
  }
  if (action == 4) {
    ReleaseCapture();
    SendMessageW((HWND)hwnd, WM_NCLBUTTONDOWN, HTBOTTOMRIGHT, 0);
  }
}
void nn6_round_window(void *hwnd) {
  if (!hwnd)
    return;
  DWORD preference = 2;
  DwmSetWindowAttribute((HWND)hwnd, 33, &preference, sizeof(preference));
  if (!previous_window_proc)
    previous_window_proc = (WNDPROC)SetWindowLongPtrW(
        (HWND)hwnd, GWLP_WNDPROC, (LONG_PTR)app_window_proc);
}
unsigned nn6_startup(const wchar_t *command, int change, wchar_t *out,
                     unsigned cap) {
  HKEY key;
  auto e = RegCreateKeyExW(
      HKEY_CURRENT_USER, L"Software\\Microsoft\\Windows\\CurrentVersion\\Run",
      0, nullptr, 0, KEY_QUERY_VALUE | KEY_SET_VALUE, nullptr, &key, nullptr);
  if (e)
    return e;
  if (change > 0)
    e = RegSetValueExW(key, L"NN6PowerPlanNative", 0, REG_SZ,
                       (const BYTE *)command, (wcslen(command) + 1) * 2);
  if (change < 0) {
    e = RegDeleteValueW(key, L"NN6PowerPlanNative");
    if (e == ERROR_FILE_NOT_FOUND)
      e = 0;
  }
  if (change == 0) {
    DWORD bytes = cap * 2;
    DWORD type = 0;
    e = RegQueryValueExW(key, L"NN6PowerPlanNative", nullptr, &type,
                         (BYTE *)out, &bytes);
    if (e == ERROR_FILE_NOT_FOUND) {
      out[0] = 0;
      e = 0;
    }
  }
  RegCloseKey(key);
  return e;
}
void *nn6_pipe(const wchar_t *name) {
  wchar_t sid[256] = {};
  if (nn6_sid(sid, 256))
    return nullptr;
  std::wstring d = L"D:P(A;;GA;;;" + std::wstring(sid) + L")";
  PSECURITY_DESCRIPTOR sd = nullptr;
  if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
          d.c_str(), SDDL_REVISION_1, &sd, nullptr))
    return nullptr;
  SECURITY_ATTRIBUTES sa{sizeof(sa), sd, FALSE};
  HANDLE h = CreateNamedPipeW(name, PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                              PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT |
                                  PIPE_REJECT_REMOTE_CLIENTS,
                              8, 65536, 65536, 0, &sa);
  LocalFree(sd);
  return h == INVALID_HANDLE_VALUE ? nullptr : h;
}
unsigned nn6_connect_pipe(void *h) {
  OVERLAPPED ov{};
  ov.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (!ov.hEvent)
    return GetLastError();
  DWORD n = 0;
  unsigned e = 0;
  if (!ConnectNamedPipe((HANDLE)h, &ov)) {
    e = GetLastError();
    if (e == ERROR_PIPE_CONNECTED)
      e = 0;
    else if (e == ERROR_IO_PENDING)
      e = GetOverlappedResult((HANDLE)h, &ov, &n, TRUE) ? 0 : GetLastError();
  }
  CloseHandle(ov.hEvent);
  return e;
}
unsigned nn6_pipe_io(void *h, void *data, unsigned size, unsigned *count,
                     bool writing) {
  OVERLAPPED ov{};
  ov.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (!ov.hEvent)
    return GetLastError();
  unsigned e = 0;
  BOOL ok = writing ? WriteFile((HANDLE)h, data, size, (DWORD *)count, &ov)
                    : ReadFile((HANDLE)h, data, size, (DWORD *)count, &ov);
  if (!ok) {
    e = GetLastError();
    if (e == ERROR_IO_PENDING)
      e = GetOverlappedResult((HANDLE)h, &ov, (DWORD *)count, TRUE)
              ? 0
              : GetLastError();
  }
  CloseHandle(ov.hEvent);
  return e;
}
void nn6_wait_process(unsigned pid) {
  HANDLE h = OpenProcess(SYNCHRONIZE, FALSE, pid);
  if (h) {
    WaitForSingleObject(h, INFINITE);
    CloseHandle(h);
  }
}
}

class ProcessSink final : public IWbemObjectSink {
  std::atomic<LONG> refs{1};
  unsigned kind;
  unsigned long long generation;

public:
  explicit ProcessSink(unsigned k) : kind(k), generation(stamp()) {}
  ULONG STDMETHODCALLTYPE AddRef() override { return ++refs; }
  ULONG STDMETHODCALLTYPE Release() override {
    LONG n = --refs;
    if (!n)
      delete this;
    return n;
  }
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void **out) override {
    if (id == IID_IUnknown || id == IID_IWbemObjectSink) {
      *out = static_cast<IWbemObjectSink *>(this);
      AddRef();
      return S_OK;
    }
    *out = nullptr;
    return E_NOINTERFACE;
  }
  HRESULT STDMETHODCALLTYPE Indicate(LONG count,
                                     IWbemClassObject **objects) override {
    for (LONG i = 0; i < count; i++) {
      VARIANT name, pid;
      VariantInit(&name);
      VariantInit(&pid);
      objects[i]->Get(L"ProcessName", 0, &name, nullptr, nullptr);
      objects[i]->Get(L"ProcessID", 0, &pid, nullptr, nullptr);
      if (name.vt != VT_BSTR) {
        VARIANT target;
        VariantInit(&target);
        if (SUCCEEDED(objects[i]->Get(L"TargetInstance", 0, &target, nullptr,
                                      nullptr)) &&
            target.vt == VT_UNKNOWN && target.punkVal) {
          IWbemClassObject *process = nullptr;
          if (SUCCEEDED(target.punkVal->QueryInterface(IID_IWbemClassObject,
                                                       (void **)&process))) {
            VariantClear(&name);
            VariantClear(&pid);
            process->Get(L"Name", 0, &name, nullptr, nullptr);
            process->Get(L"ProcessId", 0, &pid, nullptr, nullptr);
            process->Release();
          }
        }
        VariantClear(&target);
      }
      if (name.vt == VT_BSTR && event_fn)
        event_fn(kind, (unsigned)pid.uintVal, name.bstrVal, generation);
      VariantClear(&name);
      VariantClear(&pid);
    }
    return WBEM_S_NO_ERROR;
  }
  HRESULT STDMETHODCALLTYPE SetStatus(LONG flags, HRESULT result, BSTR,
                                      IWbemClassObject *) override {
    if (!stopping && flags == WBEM_STATUS_COMPLETE && FAILED(result) &&
        result != (HRESULT)WBEM_E_CALL_CANCELLED && event_fn)
      event_fn(5, (unsigned)result, L"Process event provider stopped",
               generation);
    return WBEM_S_NO_ERROR;
  }
};
static void CALLBACK foreground(HWINEVENTHOOK, DWORD, HWND hwnd, LONG, LONG,
                                DWORD, DWORD) {
  DWORD pid = 0;
  GetWindowThreadProcessId(hwnd, &pid);
  RECT r{};
  MONITORINFO m{sizeof(m)};
  bool full = false;
  if (GetWindowRect(hwnd, &r) &&
      GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &m))
    full = r.left <= m.rcMonitor.left && r.top <= m.rcMonitor.top &&
           r.right >= m.rcMonitor.right && r.bottom >= m.rcMonitor.bottom;
  if (event_fn)
    event_fn(3, pid, full ? L"fullscreen" : L"windowed", stamp());
}
// Games can acquire focus before changing their window to fullscreen.
// Observe only the foreground top-level window's bounds, never poll them.
static void CALLBACK location_changed(HWINEVENTHOOK h, DWORD event, HWND hwnd,
                                      LONG object, LONG child, DWORD thread,
                                      DWORD time) {
  if (object == OBJID_WINDOW && child == CHILDID_SELF &&
      hwnd == GetForegroundWindow())
    foreground(h, event, hwnd, object, child, thread, time);
}
static LRESULT CALLBACK power_window(HWND h, UINT msg, WPARAM w, LPARAM l) {
  if (msg == WM_POWERBROADCAST && w == PBT_POWERSETTINGCHANGE && event_fn)
    event_fn(4, 0, L"Power scheme changed", stamp());
  return DefWindowProcW(h, msg, w, l);
}
extern "C" unsigned nn6_events(EventFn callback,
                               unsigned long long generation) {
  event_generation = generation;
  event_fn = callback;
  event_thread = GetCurrentThreadId();
  MSG seed;
  PeekMessageW(&seed, nullptr, 0, 0, PM_NOREMOVE);
  HRESULT hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
  if (FAILED(hr))
    return hr;
  CoInitializeSecurity(nullptr, -1, nullptr, nullptr, RPC_C_AUTHN_LEVEL_DEFAULT,
                       RPC_C_IMP_LEVEL_IMPERSONATE, nullptr, EOAC_NONE,
                       nullptr);
  IWbemLocator *loc = nullptr;
  IWbemServices *svc = nullptr;
  ProcessSink *start = new ProcessSink(1);
  ProcessSink *stop = new ProcessSink(2);
  IWbemUnsecuredApartment *apartment = nullptr;
  IWbemObjectSink *startStub = nullptr;
  IWbemObjectSink *stopStub = nullptr;
  hr = CoCreateInstance(CLSID_WbemLocator, nullptr, CLSCTX_INPROC_SERVER,
                        IID_IWbemLocator, (void **)&loc);
  if (SUCCEEDED(hr)) {
    BSTR ns = SysAllocString(L"ROOT\\CIMV2");
    hr = loc->ConnectServer(ns, nullptr, nullptr, nullptr, 0, nullptr, nullptr,
                            &svc);
    SysFreeString(ns);
  }
  if (SUCCEEDED(hr))
    hr = CoSetProxyBlanket(svc, RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE, nullptr,
                           RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
                           nullptr, EOAC_NONE);
  if (SUCCEEDED(hr))
    hr =
        CoCreateInstance(CLSID_UnsecuredApartment, nullptr, CLSCTX_LOCAL_SERVER,
                         IID_IWbemUnsecuredApartment, (void **)&apartment);
  // Always authenticate callbacks, without changing any global WMI settings.
  if (SUCCEEDED(hr))
    hr = apartment->CreateSinkStub(start, WBEM_FLAG_UNSECAPP_CHECK_ACCESS,
                                   nullptr, &startStub);
  if (SUCCEEDED(hr))
    hr = apartment->CreateSinkStub(stop, WBEM_FLAG_UNSECAPP_CHECK_ACCESS,
                                   nullptr, &stopStub);
  BSTR lang = SysAllocString(L"WQL"),
       q1 = SysAllocString(L"SELECT * FROM Win32_ProcessStartTrace"),
       q2 = SysAllocString(L"SELECT * FROM Win32_ProcessStopTrace");
  if (SUCCEEDED(hr))
    hr = svc->ExecNotificationQueryAsync(lang, q1, WBEM_FLAG_SEND_STATUS,
                                         nullptr, startStub);
  if (SUCCEEDED(hr))
    hr = svc->ExecNotificationQueryAsync(lang, q2, WBEM_FLAG_SEND_STATUS,
                                         nullptr, stopStub);
  bool compatibility = false;
  if (hr == (HRESULT)WBEM_E_ACCESS_DENIED && svc && startStub && stopStub) {
    // Never clear a concurrent Stop request while establishing the fallback.
    svc->CancelAsyncCall(startStub);
    svc->CancelAsyncCall(stopStub);
    SysFreeString(q1);
    SysFreeString(q2);
    q1 = SysAllocString(L"SELECT * FROM __InstanceCreationEvent WITHIN 1 WHERE "
                        L"TargetInstance ISA 'Win32_Process'");
    q2 = SysAllocString(L"SELECT * FROM __InstanceDeletionEvent WITHIN 1 WHERE "
                        L"TargetInstance ISA 'Win32_Process'");
    hr = svc->ExecNotificationQueryAsync(lang, q1, WBEM_FLAG_SEND_STATUS,
                                         nullptr, startStub);
    if (SUCCEEDED(hr))
      hr = svc->ExecNotificationQueryAsync(lang, q2, WBEM_FLAG_SEND_STATUS,
                                           nullptr, stopStub);
    compatibility = SUCCEEDED(hr);
  }
  SysFreeString(lang);
  SysFreeString(q1);
  SysFreeString(q2);
  HWINEVENTHOOK hook = nullptr;
  HWINEVENTHOOK location_hook = nullptr;
  HWND hwnd = nullptr;
  HPOWERNOTIFY notify = nullptr;
  if (SUCCEEDED(hr)) {
    hook = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND,
                           nullptr, foreground, 0, 0,
                           WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
    location_hook =
        SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE,
                        EVENT_OBJECT_LOCATIONCHANGE, nullptr, location_changed,
                        0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
    WNDCLASSW wc{};
    wc.lpfnWndProc = power_window;
    wc.lpszClassName = L"NN6NativePowerEvents";
    wc.hInstance = GetModuleHandleW(nullptr);
    RegisterClassW(&wc);
    hwnd = CreateWindowExW(0, wc.lpszClassName, L"", 0, 0, 0, 0, 0,
                           HWND_MESSAGE, nullptr, wc.hInstance, nullptr);
    notify = RegisterPowerSettingNotification(hwnd, &GUID_ACTIVE_POWERSCHEME,
                                              DEVICE_NOTIFY_WINDOW_HANDLE);
    if (!stopping) {
      callback(7, 0,
               compatibility
                   ? L"Compatibility: WMI checks process starts every 1 second"
                   : L"Native WMI process start/stop traces",
               stamp());
      callback(6, 0, L"Process event listeners ready", stamp());
      foreground(nullptr, 0, GetForegroundWindow(), 0, 0, 0, 0);
    }
    MSG msg;
    while (!stopping && GetMessageW(&msg, nullptr, 0, 0) > 0) {
      TranslateMessage(&msg);
      DispatchMessageW(&msg);
    }
  }
  if (notify)
    UnregisterPowerSettingNotification(notify);
  if (hwnd)
    DestroyWindow(hwnd);
  if (hook)
    UnhookWinEvent(hook);
  if (location_hook)
    UnhookWinEvent(location_hook);
  stopping = true;
  if (svc) {
    if (startStub)
      svc->CancelAsyncCall(startStub);
    if (stopStub)
      svc->CancelAsyncCall(stopStub);
    svc->Release();
  }
  if (startStub)
    startStub->Release();
  if (stopStub)
    stopStub->Release();
  if (apartment)
    apartment->Release();
  if (loc)
    loc->Release();
  start->Release();
  stop->Release();
  event_thread = 0;
  CoUninitialize();
  return hr;
}
extern "C" void nn6_prepare_events() { stopping = false; }
extern "C" void nn6_stop_events() {
  stopping = true;
  auto id = event_thread.load();
  if (id)
    PostThreadMessageW(id, WM_QUIT, 0, 0);
}
