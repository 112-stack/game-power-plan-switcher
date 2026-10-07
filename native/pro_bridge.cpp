// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
// Native desktop integrations and an explicit, caller-approved powercfg broker.
// No automatic elevation, driver installation or power switching occurs here.
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#define _WIN32_WINNT 0x0A00
#include <windows.h>
#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdlib>
#include <commdlg.h>
#include <condition_variable>
#include <cwctype>
#include <mutex>
#include <shellapi.h>
#include <string>
#include <thread>
#include <wbemidl.h>
#include <vector>

using ProCallback = void (*)(void *, unsigned, unsigned);
static constexpr UINT TrayMessage = WM_APP + 61;
static constexpr UINT TrayConfigure = WM_APP + 62;
static constexpr UINT HotkeyConfigure = WM_APP + 63;
static constexpr UINT TrayAlreadyRunning = WM_APP + 64;
static constexpr UINT TrayLanguageConfigure = WM_APP + 65;
static constexpr UINT MonitorHotkey = 0x4e61;
static constexpr UINT OverlayHotkey = 0x4e62;
static std::atomic<unsigned> last_external_foreground{0};
static void remember_foreground(HWND window) {
  if (!window)
    return;
  wchar_t class_name[80]{};
  GetClassNameW(window, class_name, 80);
  // Desktop/taskbar activation is not a user application's foreground window.
  if (!wcscmp(class_name, L"Shell_TrayWnd") ||
      !wcscmp(class_name, L"Shell_SecondaryTrayWnd") ||
      !wcscmp(class_name, L"Progman") || !wcscmp(class_name, L"WorkerW"))
    return;
  DWORD pid = 0;
  GetWindowThreadProcessId(window, &pid);
  if (pid && pid != GetCurrentProcessId())
    last_external_foreground = pid;
}
static void CALLBACK pro_foreground_event(HWINEVENTHOOK, DWORD event,
                                          HWND window, LONG, LONG, DWORD,
                                          DWORD) {
  if (event == EVENT_SYSTEM_FOREGROUND)
    remember_foreground(window);
}

struct ProHotkeyResult {
  unsigned monitor_error = 0;
  unsigned overlay_error = 0;
  unsigned monitor_registered = 0;
  unsigned overlay_registered = 0;
};
struct ProHotkeyRequest {
  unsigned monitor_modifiers, monitor_key, overlay_modifiers, overlay_key;
  ProHotkeyResult *result;
};
struct ProTrayRequest {
  bool enabled, monitoring;
};
// Fixed storage avoids allocations/exceptions across the C ABI and window
// procedure. Captions belong to the tray thread after synchronous delivery.
static constexpr unsigned TrayLabelCount = 7, TrayLabelCapacity = 256;
struct ProTrayLanguage {
  wchar_t labels[TrayLabelCount][TrayLabelCapacity] = {
      L"Overview",
      L"Gaming plan",
      L"Default plan",
      L"Choose a power plan",
      L"Pause monitoring",
      L"Resume monitoring",
      L"Exit && restore Default"};
};
struct ProContext {
  ProCallback callback = nullptr;
  void *user = nullptr;
  std::thread thread;
  std::mutex mutex;
  std::condition_variable ready;
  HWND window = nullptr;
  unsigned startup_error = 0;
  bool started = false;
  bool tray_wanted = false;
  bool tray_added = false;
  bool monitoring = false;
  bool monitor_registered = false;
  bool overlay_registered = false;
  bool duplicate_notice_sent = false;
  ULONGLONG duplicate_notice_tick = 0;
  unsigned monitor_modifiers = 0, monitor_key = 0;
  unsigned overlay_modifiers = 0, overlay_key = 0;
  UINT taskbar_created = 0;
  HICON icon = nullptr;
  HWINEVENTHOOK foreground_hook = nullptr;
  NOTIFYICONDATAW notification{sizeof(NOTIFYICONDATAW)};
  ProTrayLanguage tray_language;
  void emit(unsigned event, unsigned value = 0) {
    if (callback)
      callback(user, event, value);
  }
};

static unsigned last_error(unsigned fallback = ERROR_GEN_FAILURE) {
  unsigned value = GetLastError();
  return value ? value : fallback;
}
static void pro_text(const std::wstring &value, wchar_t *out, unsigned cap) {
  if (out && cap) {
    wcsncpy(out, value.c_str(), cap - 1);
    out[cap - 1] = 0;
  }
}
static bool pro_dark_theme() {
  DWORD light = 0, size = sizeof(light);
  if (RegGetValueW(
          HKEY_CURRENT_USER,
          L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
          L"AppsUseLightTheme", RRF_RT_REG_DWORD, nullptr, &light,
          &size) != ERROR_SUCCESS)
    return true; // A missing preference falls back to this app's dark theme.
  return light == 0;
}
static unsigned add_tray(ProContext *context) {
  auto &note = context->notification;
  note.hWnd = context->window;
  note.uID = 1;
  note.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
  note.uCallbackMessage = TrayMessage;
  note.hIcon = context->icon;
  pro_text(context->monitoring ? L"Game Power Plan Switcher - monitoring"
                               : L"Game Power Plan Switcher - paused",
           note.szTip, 128);
  SetLastError(0);
  if (!Shell_NotifyIconW(context->tray_added ? NIM_MODIFY : NIM_ADD, &note))
    return last_error();
  context->tray_added = true;
  note.uVersion = NOTIFYICON_VERSION_4;
  if (!Shell_NotifyIconW(NIM_SETVERSION, &note)) {
    auto error = last_error();
    Shell_NotifyIconW(NIM_DELETE, &note);
    context->tray_added = false;
    return error;
  }
  return 0;
}
static void remove_tray(ProContext *context) {
  if (context->tray_added)
    Shell_NotifyIconW(NIM_DELETE, &context->notification);
  context->tray_added = false;
}
static void notify_already_running(ProContext *context) {
  // A SHOW request must not create an unwanted icon or retry a failed tray
  // registration. All state here belongs to the native message thread.
  if (!context->tray_wanted || !context->tray_added)
    return;
  const ULONGLONG now = GetTickCount64();
  if (context->duplicate_notice_sent &&
      now - context->duplicate_notice_tick < 5000)
    return;
  context->duplicate_notice_sent = true;
  context->duplicate_notice_tick = now;
  // Use a separate structure so ordinary tooltip refreshes cannot replay the
  // balloon. REALTIME discards delayed notices; Windows controls visibility.
  NOTIFYICONDATAW note{sizeof(NOTIFYICONDATAW)};
  note.hWnd = context->window;
  note.uID = context->notification.uID;
  note.uFlags = NIF_INFO | NIF_REALTIME;
  note.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND | NIIF_RESPECT_QUIET_TIME;
  pro_text(L"Game Power Plan Switcher", note.szInfoTitle, 64);
  pro_text(L"Game Power Plan Switcher is already running. Bringing existing "
           L"window to front.",
           note.szInfo, 256);
  SetLastError(0);
  if (!Shell_NotifyIconW(NIM_MODIFY, &note))
    context->emit(8, last_error());
}
static void tray_menu(ProContext *context, LPARAM position) {
  HMENU menu = CreatePopupMenu();
  if (!menu) {
    context->emit(8, last_error());
    return;
  }
  const auto &labels = context->tray_language.labels;
  AppendMenuW(menu, MF_STRING, 1, labels[0]);
  AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
  HMENU profiles = CreatePopupMenu();
  if (!profiles) {
    DestroyMenu(menu);
    context->emit(8, last_error());
    return;
  }
  AppendMenuW(profiles, MF_STRING, 2, labels[1]);
  AppendMenuW(profiles, MF_STRING, 3, labels[2]);
  AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(profiles), labels[3]);
  AppendMenuW(menu, MF_STRING, 4, context->monitoring ? labels[4] : labels[5]);
  AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(menu, MF_STRING, 5, labels[6]);
  POINT point{static_cast<short>(LOWORD(position)),
              static_cast<short>(HIWORD(position))};
  if (point.x == -1 && point.y == -1)
    GetCursorPos(&point);
  SetForegroundWindow(context->window);
  UINT command =
      TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                     point.x, point.y, 0, context->window, nullptr);
  DestroyMenu(menu);
  PostMessageW(context->window, WM_NULL, 0, 0);
  if (command)
    context->emit(command);
  else
    Shell_NotifyIconW(NIM_SETFOCUS, &context->notification);
}
static void configure_hotkey(HWND window, unsigned id, unsigned modifiers,
                             unsigned key, bool &registered, unsigned &old_mods,
                             unsigned &old_key, unsigned &error) {
  error = 0;
  if (registered && modifiers == old_mods && key == old_key)
    return;
  if (registered) {
    if (!UnregisterHotKey(window, id)) {
      error = last_error();
      return; // Do not lose track of a registration Windows did not release.
    }
    registered = false;
  }
  old_mods = modifiers;
  old_key = key;
  if (key) {
    SetLastError(0);
    if (RegisterHotKey(window, id, modifiers | MOD_NOREPEAT, key))
      registered = true;
    else
      error = last_error(ERROR_HOTKEY_ALREADY_REGISTERED);
  }
}
static LRESULT CALLBACK pro_window_proc(HWND window, UINT message, WPARAM w,
                                        LPARAM l) {
  auto *context =
      reinterpret_cast<ProContext *>(GetWindowLongPtrW(window, GWLP_USERDATA));
  if (message == WM_NCCREATE) {
    context = static_cast<ProContext *>(
        reinterpret_cast<CREATESTRUCTW *>(l)->lpCreateParams);
    SetWindowLongPtrW(window, GWLP_USERDATA,
                      reinterpret_cast<LONG_PTR>(context));
    context->window = window;
  }
  if (!context)
    return DefWindowProcW(window, message, w, l);
  if (message == context->taskbar_created && context->taskbar_created) {
    context->tray_added = false;
    if (context->tray_wanted) {
      if (auto error = add_tray(context))
        context->emit(8, error);
    }
    return 0;
  }
  switch (message) {
  case TrayMessage: {
    UINT event = LOWORD(l); // NOTIFYICON_VERSION_4 packs event and icon ID.
    if (event == WM_CONTEXTMENU)
      tray_menu(context, static_cast<LPARAM>(w));
    else if (event == NIN_SELECT || event == NIN_KEYSELECT ||
             event == NIN_BALLOONUSERCLICK)
      context->emit(1);
    return 0;
  }
  case TrayConfigure: {
    auto *request = reinterpret_cast<ProTrayRequest *>(l);
    context->tray_wanted = request->enabled;
    context->monitoring = request->monitoring;
    if (!request->enabled) {
      remove_tray(context);
      return 0;
    }
    return add_tray(context);
  }
  case TrayLanguageConfigure: {
    const auto *request = reinterpret_cast<const ProTrayLanguage *>(l);
    if (!request)
      return ERROR_INVALID_PARAMETER;
    context->tray_language = *request;
    return 0;
  }
  case HotkeyConfigure: {
    auto *request = reinterpret_cast<ProHotkeyRequest *>(l);
    auto &result = *request->result;
    configure_hotkey(window, MonitorHotkey, request->monitor_modifiers,
                     request->monitor_key, context->monitor_registered,
                     context->monitor_modifiers, context->monitor_key,
                     result.monitor_error);
    configure_hotkey(window, OverlayHotkey, request->overlay_modifiers,
                     request->overlay_key, context->overlay_registered,
                     context->overlay_modifiers, context->overlay_key,
                     result.overlay_error);
    result.monitor_registered = context->monitor_registered;
    result.overlay_registered = context->overlay_registered;
    return 0;
  }
  case TrayAlreadyRunning:
    notify_already_running(context);
    return 0;
  case WM_HOTKEY:
    if (w == MonitorHotkey)
      context->emit(
          9); // Ctrl+Alt+P switches plans; tray Pause remains event 4.
    else if (w == OverlayHotkey)
      context->emit(6);
    return 0;
  case WM_SETTINGCHANGE:
    context->emit(7, pro_dark_theme() ? 1 : 0);
    // Re-check the Windows display-language API, not keyboard layout. Defer
    // actual GUI work through the existing asynchronous Rust callback.
    context->emit(10);
    return 0;
  case WM_THEMECHANGED:
    context->emit(7, pro_dark_theme() ? 1 : 0);
    return 0;
  case WM_CLOSE:
    DestroyWindow(window);
    return 0;
  case WM_DESTROY:
    remove_tray(context);
    if (context->monitor_registered)
      UnregisterHotKey(window, MonitorHotkey);
    if (context->overlay_registered)
      UnregisterHotKey(window, OverlayHotkey);
    context->monitor_registered = context->overlay_registered = false;
    PostQuitMessage(0);
    return 0;
  }
  return DefWindowProcW(window, message, w, l);
}
extern "C" void *nn6_pro_start(ProCallback callback, void *user,
                               unsigned *error) {
  *error = 0;
  ProContext *context = nullptr;
  try {
    context = new ProContext;
    context->callback = callback;
    context->user = user;
    context->thread = std::thread([context] {
      // Capture before the main Slint window is shown, then retain the most
      // recent external foreground app while NN6's picker is in the foreground.
      remember_foreground(GetForegroundWindow());
      context->foreground_hook =
          SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND,
                          nullptr, pro_foreground_event, 0, 0,
                          WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
      HINSTANCE instance = GetModuleHandleW(nullptr);
      WNDCLASSW cls{};
      cls.hInstance = instance;
      cls.lpfnWndProc = pro_window_proc;
      cls.lpszClassName = L"NN6.SystemIntegration.2.3";
      unsigned startup_error = 0;
      if (!RegisterClassW(&cls) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS)
        startup_error = last_error();
      context->taskbar_created = RegisterWindowMessageW(L"TaskbarCreated");
      if (!startup_error) {
        context->icon = static_cast<HICON>(
            LoadImageW(instance, MAKEINTRESOURCEW(1), IMAGE_ICON,
                       GetSystemMetrics(SM_CXSMICON),
                       GetSystemMetrics(SM_CYSMICON), LR_DEFAULTCOLOR));
        if (!context->icon)
          startup_error = last_error();
      }
      // A hidden top-level tool window receives broadcasts such as
      // TaskbarCreated and WM_SETTINGCHANGE; HWND_MESSAGE deliberately would
      // not receive them.
      if (!startup_error &&
          !CreateWindowExW(WS_EX_TOOLWINDOW, cls.lpszClassName,
                           L"Game Power Plan Switcher background integration",
                           WS_POPUP, 0, 0, 0, 0, nullptr, nullptr, instance,
                           context))
        startup_error = last_error();
      {
        std::lock_guard<std::mutex> lock(context->mutex);
        context->startup_error = startup_error;
        context->started = true;
      }
      context->ready.notify_one();
      if (!startup_error) {
        MSG message;
        while (GetMessageW(&message, nullptr, 0, 0) > 0) {
          TranslateMessage(&message);
          DispatchMessageW(&message);
        }
        if (IsWindow(context->window))
          DestroyWindow(context->window);
      }
      if (context->icon)
        DestroyIcon(context->icon);
      if (context->foreground_hook)
        UnhookWinEvent(context->foreground_hook);
    });
    std::unique_lock<std::mutex> lock(context->mutex);
    context->ready.wait(lock, [context] { return context->started; });
    *error = context->startup_error;
    lock.unlock();
    if (*error) {
      context->thread.join();
      delete context;
      return nullptr;
    }
    return context;
  } catch (...) {
    *error = ERROR_NOT_ENOUGH_MEMORY;
    delete context;
    return nullptr;
  }
}
extern "C" void nn6_pro_close(void *handle) {
  if (auto *context = static_cast<ProContext *>(handle)) {
    PostMessageW(context->window, WM_CLOSE, 0, 0);
    if (context->thread.joinable())
      context->thread.join();
    delete context;
  }
}
extern "C" unsigned nn6_pro_tray(void *handle, bool enabled, bool monitoring) {
  auto *context = static_cast<ProContext *>(handle);
  if (!context || !IsWindow(context->window))
    return ERROR_INVALID_WINDOW_HANDLE;
  ProTrayRequest request{enabled, monitoring};
  return static_cast<unsigned>(SendMessageW(
      context->window, TrayConfigure, 0, reinterpret_cast<LPARAM>(&request)));
}
extern "C" unsigned nn6_pro_tray_language(void *handle, const wchar_t *labels,
                                          unsigned count, unsigned capacity) {
  auto *context = static_cast<ProContext *>(handle);
  if (!context || !IsWindow(context->window))
    return ERROR_INVALID_WINDOW_HANDLE;
  if (!labels || count != TrayLabelCount || capacity != TrayLabelCapacity)
    return ERROR_INVALID_PARAMETER;
  ProTrayLanguage request;
  for (unsigned row = 0; row < TrayLabelCount; ++row) {
    const auto *start = labels + row * TrayLabelCapacity;
    if (!start[0] || std::find(start, start + TrayLabelCapacity, L'\0') ==
                         start + TrayLabelCapacity)
      return ERROR_INVALID_PARAMETER;
    std::copy(start, start + TrayLabelCapacity, request.labels[row]);
  }
  // SendMessageW completes the copy before this stack request is destroyed.
  // Never convert this to PostMessageW with caller-owned storage.
  return static_cast<unsigned>(
      SendMessageW(context->window, TrayLanguageConfigure, 0,
                   reinterpret_cast<LPARAM>(&request)));
}
extern "C" unsigned nn6_pro_notify_already_running(void *handle) {
  auto *context = static_cast<ProContext *>(handle);
  if (!context || !IsWindow(context->window))
    return ERROR_INVALID_WINDOW_HANDLE;
  // No caller-owned payload: queue the request without blocking Slint on the
  // shell. Subsequent shell failures use the existing tray-error callback.
  SetLastError(0);
  return PostMessageW(context->window, TrayAlreadyRunning, 0, 0) ? 0
                                                                 : last_error();
}
extern "C" unsigned nn6_pro_hotkeys(void *handle, unsigned monitor_modifiers,
                                    unsigned monitor_key,
                                    unsigned overlay_modifiers,
                                    unsigned overlay_key,
                                    ProHotkeyResult *result) {
  auto *context = static_cast<ProContext *>(handle);
  if (!context || !result || !IsWindow(context->window))
    return ERROR_INVALID_WINDOW_HANDLE;
  *result = {};
  ProHotkeyRequest request{monitor_modifiers, monitor_key, overlay_modifiers,
                           overlay_key, result};
  SendMessageW(context->window, HotkeyConfigure, 0,
               reinterpret_cast<LPARAM>(&request));
  return 0;
}
extern "C" bool nn6_pro_dark() { return pro_dark_theme(); }
static int CALLBACK found_font(const LOGFONTW *, const TEXTMETRICW *, DWORD,
                               LPARAM data) {
  *reinterpret_cast<bool *>(data) = true;
  return 0;
}
extern "C" unsigned nn6_pro_font(wchar_t *out, unsigned cap) {
  if (!out || !cap)
    return ERROR_INVALID_PARAMETER;
  HDC dc = GetDC(nullptr);
  if (!dc)
    return last_error();
  // Resolve only installed families. SF Pro is optional and never downloaded
  // or bundled; the ordinary Windows system font remains the final fallback.
  for (const wchar_t *family :
       {L"Segoe UI Variable", L"Segoe UI Variable Text", L"Inter", L"SF Pro",
        L"SF Pro Display", L"SF Pro Text", L"Segoe UI"}) {
    LOGFONTW font{};
    font.lfCharSet = DEFAULT_CHARSET;
    pro_text(family, font.lfFaceName, LF_FACESIZE);
    bool found = false;
    EnumFontFamiliesExW(dc, &font, found_font, reinterpret_cast<LPARAM>(&found),
                        0);
    if (found) {
      ReleaseDC(nullptr, dc);
      pro_text(family, out, cap);
      return 0;
    }
  }
  ReleaseDC(nullptr, dc);
  NONCLIENTMETRICSW metrics{sizeof(metrics)};
  if (SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, sizeof(metrics), &metrics,
                            0))
    pro_text(metrics.lfMessageFont.lfFaceName, out, cap);
  else
    pro_text(L"Segoe UI", out, cap);
  return 0;
}

struct ProSensors {
  double cpu_c = -1, gpu_c = -1, package_w = -1, fan_rpm = -1;
  unsigned readings = 0;
  unsigned provider_connected = 0;
  wchar_t provider[96]{};
  wchar_t cpu_label[128]{}, gpu_label[128]{}, power_label[128]{},
      fan_label[128]{};
};
template <class T> struct ComPtr {
  T *value = nullptr;
  ~ComPtr() {
    if (value)
      value->Release();
  }
  T **out() { return &value; }
  T *operator->() const { return value; }
};
struct ProCom {
  HRESULT status;
  explicit ProCom(DWORD mode = COINIT_MULTITHREADED)
      : status(CoInitializeEx(nullptr, mode)) {}
  ~ProCom() {
    if (SUCCEEDED(status))
      CoUninitialize();
  }
};
// Cancellation applies only to this worker's outgoing COM calls. It never kills
// a provider or a Windows service. Providers may not honor cancellation, so
// ConnectServer also uses the documented maximum-wait flag and no UI waits
// here.
struct ProCancellation {
  bool enabled = SUCCEEDED(CoEnableCallCancellation(nullptr));
  std::atomic<bool> expired = false;
  DWORD target = GetCurrentThreadId();
  HANDLE timer = nullptr;
  static void CALLBACK cancel(void *parameter, BOOLEAN) {
    auto *self = static_cast<ProCancellation *>(parameter);
    self->expired = true;
    HRESULT initialized = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    CoCancelCall(self->target, 0);
    if (SUCCEEDED(initialized))
      CoUninitialize();
  }
  ProCancellation() {
    // Reuse Windows' timer-queue worker rather than constructing a dedicated
    // watchdog thread for every WMI sample. Cancellation remains best effort.
    if (enabled)
      CreateTimerQueueTimer(&timer, nullptr, cancel, this, 6000, 0,
                            WT_EXECUTEONLYONCE);
  }
  ~ProCancellation() {
    if (timer)
      DeleteTimerQueueTimer(nullptr, timer, INVALID_HANDLE_VALUE);
    if (enabled)
      CoDisableCallCancellation(nullptr);
  }
};
struct ProBstr {
  BSTR value;
  explicit ProBstr(const wchar_t *text) : value(SysAllocString(text)) {}
  ~ProBstr() { SysFreeString(value); }
};
static std::wstring string_property(IWbemClassObject *object,
                                    const wchar_t *key) {
  VARIANT value;
  VariantInit(&value);
  std::wstring result;
  if (SUCCEEDED(object->Get(key, 0, &value, nullptr, nullptr)) &&
      value.vt == VT_BSTR && value.bstrVal)
    result = value.bstrVal;
  VariantClear(&value);
  return result;
}
static double number_property(IWbemClassObject *object, const wchar_t *key) {
  VARIANT value, number;
  VariantInit(&value);
  VariantInit(&number);
  double result = -1;
  if (SUCCEEDED(object->Get(key, 0, &value, nullptr, nullptr)) &&
      value.vt != VT_NULL && value.vt != VT_EMPTY &&
      SUCCEEDED(VariantChangeType(&number, &value, 0, VT_R8)) &&
      std::isfinite(number.dblVal))
    result = number.dblVal;
  VariantClear(&number);
  VariantClear(&value);
  return result;
}
static std::wstring lower(std::wstring value) {
  std::transform(value.begin(), value.end(), value.begin(), towlower);
  return value;
}
static bool has(const std::wstring &value, const wchar_t *word) {
  return value.find(word) != std::wstring::npos;
}
static constexpr bool generic_fan_allowed(bool gpu, double value) {
  return !gpu && value >= 0 && value <= 50000;
}
static_assert(!generic_fan_allowed(true, 1400));
static_assert(generic_fan_allowed(false, 0));
static_assert(!generic_fan_allowed(false, -1));
static unsigned read_pro_sensors(ProSensors *out) {
  if (!out)
    return ERROR_INVALID_PARAMETER;
  *out = {};
  ProCom apartment;
  if (FAILED(apartment.status) && apartment.status != RPC_E_CHANGED_MODE)
    return apartment.status;
  ProCancellation cancellation;
  ComPtr<IWbemLocator> locator;
  HRESULT result = CoCreateInstance(CLSID_WbemLocator, nullptr,
                                    CLSCTX_INPROC_SERVER, IID_IWbemLocator,
                                    reinterpret_cast<void **>(locator.out()));
  if (FAILED(result))
    return result;
  HRESULT last = WBEM_E_INVALID_NAMESPACE;
  for (const wchar_t *provider :
       {L"ROOT\\LibreHardwareMonitor", L"ROOT\\OpenHardwareMonitor"}) {
    if (cancellation.expired)
      return ERROR_TIMEOUT;
    ComPtr<IWbemServices> service;
    ProBstr name(provider);
    result = locator->ConnectServer(name.value, nullptr, nullptr, nullptr,
                                    WBEM_FLAG_CONNECT_USE_MAX_WAIT, nullptr,
                                    nullptr, service.out());
    if (FAILED(result)) {
      last = result;
      continue;
    }
    result =
        CoSetProxyBlanket(service.value, RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE,
                          nullptr, RPC_C_AUTHN_LEVEL_CALL,
                          RPC_C_IMP_LEVEL_IMPERSONATE, nullptr, EOAC_NONE);
    if (FAILED(result)) {
      last = result;
      continue;
    }
    ComPtr<IEnumWbemClassObject> items;
    ProBstr language(L"WQL");
    ProBstr query(
        L"SELECT Name,Identifier,Parent,SensorType,Value FROM Sensor");
    result = service->ExecQuery(language.value, query.value,
                                WBEM_FLAG_FORWARD_ONLY |
                                    WBEM_FLAG_RETURN_IMMEDIATELY,
                                nullptr, items.out());
    if (FAILED(result)) {
      last = result;
      continue;
    }
    ProSensors candidate;
    candidate.provider_connected = 1;
    pro_text(provider, candidate.provider, 96);
    int cpu_score = -1, gpu_score = -1, power_score = -1, fan_score = -1;
    ULONGLONG deadline = GetTickCount64() + 2000;
    unsigned count = 0;
    while (count++ < 4096 && GetTickCount64() < deadline &&
           !cancellation.expired) {
      ComPtr<IWbemClassObject> object;
      ULONG fetched = 0;
      result = items->Next(200, 1, object.out(), &fetched);
      if (result == WBEM_S_TIMEDOUT)
        continue;
      if (FAILED(result)) {
        last = result;
        break;
      }
      if (!fetched)
        break;
      auto display = string_property(object.value, L"Name");
      auto sensor = lower(string_property(object.value, L"SensorType"));
      auto identifier = lower(string_property(object.value, L"Identifier"));
      auto parent = lower(string_property(object.value, L"Parent"));
      auto label = lower(display);
      auto hardware = parent + L" " + identifier;
      bool cpu = has(hardware, L"cpu") || has(label, L"cpu");
      bool gpu = has(hardware, L"gpu") || has(label, L"gpu");
      double value = number_property(object.value, L"Value");
      if (value < 0 || !std::isfinite(value))
        continue;
      ++candidate.readings;
      auto select = [&](int score, int &best, double &target,
                        wchar_t *caption) {
        if (score > best || (score == best && value > target)) {
          best = score;
          target = value;
          pro_text(display + L" [" + identifier + L"]", caption, 128);
        }
      };
      if (sensor == L"temperature" && value <= 150) {
        if (cpu)
          select(has(label, L"package") ? 100
                 : has(label, L"tctl")  ? 90
                 : has(label, L"core")  ? 60
                                        : 20,
                 cpu_score, candidate.cpu_c, candidate.cpu_label);
        if (gpu)
          select(has(label, L"core")                                 ? 100
                 : has(label, L"hot spot") || has(label, L"hotspot") ? 60
                                                                     : 30,
                 gpu_score, candidate.gpu_c, candidate.gpu_label);
      } else if (sensor == L"power" && cpu && value <= 2000 &&
                 (has(label, L"package") || has(label, L"cpu total"))) {
        select(has(label, L"package") ? 100 : 80, power_score,
               candidate.package_w, candidate.power_label);
      } else if (sensor == L"fan" && generic_fan_allowed(gpu, value)) {
        // This field is exclusively CPU/board fan RPM. A WMI GPU fan may refer
        // to a different adapter than our direct NVML/ADL device; never
        // silently use it to fill that device's unsupported fan reading.
        select(cpu ? 100 : 20, fan_score, candidate.fan_rpm,
               candidate.fan_label);
      }
    }
    if (cancellation.expired)
      return ERROR_TIMEOUT;
    if (SUCCEEDED(result)) {
      *out = candidate;
      // Prefer a provider with useful values; a registered but stopped provider
      // can have an empty Sensor class, so try the second known namespace too.
      if (candidate.cpu_c >= 0 || candidate.gpu_c >= 0 ||
          candidate.package_w >= 0 || candidate.fan_rpm >= 0)
        return 0;
      last = WBEM_E_NOT_FOUND;
    }
  }
  return out->provider_connected ? 0 : static_cast<unsigned>(last);
}
extern "C" unsigned nn6_pro_sensors(ProSensors *out) {
  try {
    return read_pro_sensors(out);
  } catch (...) {
    if (out)
      *out = {};
    return ERROR_NOT_ENOUGH_MEMORY;
  }
}

// Read-only AMD ADL8 PMLog bridge. ABI layouts and IDs are from AMD's public
// display-library/include/adl_structures.h and adl_defines.h (MIT); see the
// third-party notice. No clocks, fan curves, voltage or power limits are set.
// https://gpuopen-librariesandsdks.github.io/adl/group__OVERDRIVE8API.html
struct AdlAdapterInfo {
  int size, index;
  char udid[256];
  int bus, device, function, vendor;
  char name[256], display[256];
  int present, exists;
  char driver[256], driver_ext[256], pnp[256];
  int os_index;
};
struct AdlSensorValue {
  int supported, value;
};
struct AdlPmLog {
  int size;
  AdlSensorValue sensors[256];
};
static_assert(sizeof(AdlAdapterInfo) == 1572);
static_assert(sizeof(AdlPmLog) == 2052);
struct ProGpuSensors {
  double temperature = -1, utilization = -1, fan_percent = -1, fan_rpm = -1;
  wchar_t name[128]{};
  unsigned connected = 0;
};
using AdlAllocate = void *(__stdcall *)(int);
static void *__stdcall adl_allocate(int size) {
  return size > 0 && size <= 16 * 1024 * 1024 ? std::malloc(size) : nullptr;
}
struct ProAdl {
  HMODULE module = nullptr;
  void *context = nullptr;
  int adapter = -1;
  std::wstring name;
  unsigned device = 0;
  void *shared = nullptr;
  bool started = false;
  using Create = int (*)(AdlAllocate, int, void **);
  using Destroy = int (*)(void *);
  using Count = int (*)(void *, int *);
  using Info = int (*)(void *, AdlAdapterInfo *, int);
  using Support = int (*)(void *, int, int *, int);
  using DeviceCreate = int (*)(void *, int, unsigned *);
  using DeviceDestroy = int (*)(void *, unsigned);
  using Start = int (*)(void *, int, int, int, int *, unsigned *, void **, int);
  using Read = int (*)(void *, int, int, int *, void **, AdlPmLog *);
  using Stop = int (*)(void *, int, unsigned *);
  Destroy destroy = nullptr;
  DeviceCreate device_create = nullptr;
  DeviceDestroy device_destroy = nullptr;
  Start start = nullptr;
  Read read = nullptr;
  Stop stop = nullptr;
  int sensors[4]{8, 14, 15, 19}; // edge Celsius, RPM, fan %, graphics busy %
  template <class T> T symbol(const char *key) {
    return reinterpret_cast<T>(GetProcAddress(module, key));
  }
  unsigned initialize() {
    wchar_t directory[32768]{};
    UINT count = GetSystemDirectoryW(directory, 32768);
    if (!count || count >= 32768)
      return ERROR_PATH_NOT_FOUND;
    std::wstring path = std::wstring(directory) + L"\\atiadlxx.dll";
    module = LoadLibraryExW(path.c_str(), nullptr,
                            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR |
                                LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (!module)
      return last_error();
    auto create = symbol<Create>("ADL2_Main_Control_Create");
    destroy = symbol<Destroy>("ADL2_Main_Control_Destroy");
    auto get_count = symbol<Count>("ADL2_Adapter_NumberOfAdapters_Get");
    auto get_info = symbol<Info>("ADL2_Adapter_AdapterInfo_Get");
    auto support = symbol<Support>("ADL2_Overdrive8_PMLog_ShareMemory_Support");
    device_create = symbol<DeviceCreate>("ADL2_Device_PMLog_Device_Create");
    device_destroy = symbol<DeviceDestroy>("ADL2_Device_PMLog_Device_Destroy");
    start = symbol<Start>("ADL2_Overdrive8_PMLog_ShareMemory_Start");
    read = symbol<Read>("ADL2_Overdrive8_PMLog_ShareMemory_Read");
    stop = symbol<Stop>("ADL2_Overdrive8_PMLog_ShareMemory_Stop");
    if (!create || !destroy || !get_count || !get_info || !support ||
        !device_create || !device_destroy || !start || !read || !stop)
      return ERROR_NOT_SUPPORTED;
    if (create(adl_allocate, 1, &context) != 0 || !context)
      return ERROR_NOT_READY;
    int total = 0;
    if (get_count(context, &total) != 0 || total < 1 || total > 64)
      return ERROR_NOT_FOUND;
    std::vector<AdlAdapterInfo> adapters(static_cast<size_t>(total));
    for (auto &item : adapters)
      item.size = sizeof(item);
    if (get_info(context, adapters.data(), total * sizeof(AdlAdapterInfo)) != 0)
      return ERROR_NOT_READY;
    for (const auto &item : adapters) {
      if ((item.vendor != 1002 && item.vendor != 0x1002) || !item.exists ||
          item.bus < 0)
        continue;
      int available = 0;
      if (support(context, item.index, &available, 0) != 0 || !available)
        continue;
      adapter = item.index;
      wchar_t caption[257]{};
      int length = 0;
      while (length < 256 && item.name[length])
        ++length;
      MultiByteToWideChar(CP_ACP, 0, item.name, length, caption, 256);
      name = std::wstring(caption) + L" [ADL adapter " +
             std::to_wstring(adapter) + L"]";
      return 0;
    }
    return ERROR_NOT_SUPPORTED;
  }
  void suspend() {
    if (started)
      stop(context, adapter, &device);
    started = false;
    shared = nullptr;
    if (device)
      device_destroy(context, device);
    device = 0;
  }
  unsigned sample(ProGpuSensors *out) {
    *out = {};
    pro_text(name, out->name, 128);
    out->connected = context && adapter >= 0;
    if (!out->connected)
      return ERROR_NOT_READY;
    if (!started) {
      if (device_create(context, adapter, &device) != 0 || !device) {
        suspend();
        return ERROR_NOT_READY;
      }
      int result =
          start(context, adapter, 1000, 4, sensors, &device, &shared, 0);
      started = result == 0;
      if (!started || !shared) {
        suspend();
        return ERROR_NOT_READY;
      }
      // First observation is warm-up, never a manufactured 0-valued sample.
      return ERROR_RETRY;
    }
    AdlPmLog data{};
    data.size = sizeof(data);
    if (read(context, adapter, 4, sensors, &shared, &data) != 0) {
      suspend();
      return ERROR_NOT_READY;
    }
    auto value = [&](int id, int maximum) -> double {
      const auto &entry = data.sensors[id];
      return entry.supported && entry.value >= 0 && entry.value <= maximum
                 ? static_cast<double>(entry.value)
                 : -1.;
    };
    out->temperature = value(8, 150);
    out->utilization = value(19, 100);
    out->fan_percent = value(15, 100);
    out->fan_rpm = value(14, 50000);
    return 0;
  }
  ~ProAdl() {
    suspend();
    if (context && destroy)
      destroy(context);
    if (module)
      FreeLibrary(module);
  }
};
extern "C" void *nn6_adl_open(unsigned *error) {
  ProAdl *reader = nullptr;
  try {
    reader = new ProAdl;
    *error = reader->initialize();
    if (*error) {
      delete reader;
      return nullptr;
    }
    return reader;
  } catch (...) {
    delete reader;
    *error = ERROR_NOT_ENOUGH_MEMORY;
    return nullptr;
  }
}
extern "C" unsigned nn6_adl_sample(void *handle, ProGpuSensors *out) {
  if (!handle || !out)
    return ERROR_INVALID_PARAMETER;
  try {
    return static_cast<ProAdl *>(handle)->sample(out);
  } catch (...) {
    *out = {};
    return ERROR_NOT_ENOUGH_MEMORY;
  }
}
extern "C" void nn6_adl_suspend(void *handle) {
  if (handle)
    static_cast<ProAdl *>(handle)->suspend();
}
extern "C" void nn6_adl_close(void *handle) {
  delete static_cast<ProAdl *>(handle);
}

extern "C" unsigned nn6_pro_file_dialog(void *owner, bool save, unsigned kind,
                                        wchar_t *out, unsigned cap) {
  if (!out || cap < 260)
    return ERROR_INSUFFICIENT_BUFFER;
  // Dedicated async picker workers use STA for Explorer/common-dialog COM.
  ProCom apartment(COINIT_APARTMENTTHREADED);
  if (FAILED(apartment.status) && apartment.status != RPC_E_CHANGED_MODE)
    return apartment.status;
  out[0] = 0;
  const wchar_t *filter = kind == 1   ? L"CSV diagnostic log\0*.csv\0\0"
                          : kind == 2 ? L"JSON diagnostic log\0*.json\0\0"
                          : kind == 3 ? L"Text diagnostic log\0*.log;*.txt\0\0"
                                      : L"Windows power scheme\0*.pow\0\0";
  const wchar_t *extension = kind == 1   ? L"csv"
                             : kind == 2 ? L"json"
                             : kind == 3 ? L"log"
                                         : L"pow";
  OPENFILENAMEW dialog{sizeof(dialog)};
  dialog.hwndOwner = static_cast<HWND>(owner);
  dialog.lpstrFilter = filter;
  dialog.lpstrFile = out;
  dialog.nMaxFile = cap;
  dialog.lpstrDefExt = extension;
  dialog.lpstrTitle =
      save ? L"Export Game Power Plan Switcher data" : L"Open file";
  dialog.Flags = OFN_EXPLORER | OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST |
                 (save ? OFN_OVERWRITEPROMPT : OFN_FILEMUSTEXIST);
  BOOL accepted = save ? GetSaveFileNameW(&dialog) : GetOpenFileNameW(&dialog);
  if (!accepted) {
    auto error = CommDlgExtendedError();
    return error ? error : ERROR_CANCELLED;
  }
  return 0;
}
extern "C" unsigned nn6_pro_process_path(unsigned pid, wchar_t *out,
                                         unsigned cap) {
  if (!out || !cap)
    return ERROR_INVALID_PARAMETER;
  out[0] = 0;
  HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
  if (!process)
    return last_error();
  DWORD length = cap;
  BOOL ok = QueryFullProcessImageNameW(process, 0, out, &length);
  unsigned error = ok ? 0 : last_error();
  CloseHandle(process);
  return error;
}
extern "C" unsigned nn6_pro_last_external_foreground_pid() {
  return last_external_foreground.load();
}
extern "C" unsigned nn6_pro_elevated_powercfg(const wchar_t *parameters,
                                              unsigned *exit_code) {
  if (!parameters || !exit_code || wcsnlen(parameters, 30002) > 30000)
    return ERROR_INVALID_PARAMETER;
  *exit_code = STILL_ACTIVE;
  // Always invoked by an explicit user action on a dedicated worker. A runas
  // security prompt remains visible even though the powercfg console is hidden.
  ProCom apartment(COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
  if (FAILED(apartment.status))
    return apartment.status;
  try {
    wchar_t directory[32768]{};
    UINT size = GetSystemDirectoryW(directory, 32768);
    if (!size)
      return last_error();
    if (size >= 32768)
      return ERROR_INSUFFICIENT_BUFFER;
    // GetSystemDirectoryW is an OS API, never a caller-controlled PATH or
    // SystemRoot lookup. This executable is the only target accepted here.
    std::wstring executable(directory);
    executable += L"\\powercfg.exe";
    SHELLEXECUTEINFOW execute{sizeof(execute)};
    execute.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC |
                    SEE_MASK_FLAG_NO_UI | SEE_MASK_UNICODE;
    execute.lpVerb = L"runas";
    execute.lpFile = executable.c_str();
    execute.lpParameters = parameters;
    execute.lpDirectory = directory;
    execute.nShow = SW_HIDE;
    SetLastError(0);
    if (!ShellExecuteExW(&execute))
      return last_error(); // Includes ERROR_CANCELLED for denied UAC approval.
    if (!execute.hProcess)
      return ERROR_INVALID_HANDLE; // Never report unobserved completion.
    DWORD wait = WaitForSingleObject(execute.hProcess, 30000);
    DWORD result = 0;
    DWORD process_exit = STILL_ACTIVE;
    if (wait == WAIT_TIMEOUT)
      result = ERROR_TIMEOUT;
    else if (wait != WAIT_OBJECT_0)
      result = last_error();
    else if (!GetExitCodeProcess(execute.hProcess, &process_exit))
      result = last_error();
    *exit_code = static_cast<unsigned>(process_exit);
    // A timeout releases only our handle. It must not terminate an elevated
    // tool that may still be committing the user's requested file operation.
    CloseHandle(execute.hProcess);
    return result;
  } catch (...) {
    return ERROR_NOT_ENOUGH_MEMORY;
  }
}
static unsigned nn6_pro_overlay_position(void *main_window, unsigned width,
                                         unsigned height, unsigned corner,
                                         int *x, int *y) {
  HWND main = static_cast<HWND>(main_window);
  if (!IsWindow(main) || !x || !y || corner > 3)
    return ERROR_INVALID_PARAMETER;
  MONITORINFO monitor{sizeof(monitor)};
  if (!GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST),
                       &monitor))
    return last_error();
  UINT dpi = GetDpiForWindow(main);
  if (!dpi)
    dpi = 96;
  int margin = MulDiv(16, dpi, 96);
  int w = MulDiv(static_cast<int>(std::min(width, 10000u)), dpi, 96);
  int h = MulDiv(static_cast<int>(std::min(height, 10000u)), dpi, 96);
  const RECT &area = monitor.rcWork;
  *x = (corner == 1 || corner == 3)
           ? std::max(area.left, area.right - w - margin)
           : area.left + margin;
  *y = corner >= 2 ? std::max(area.top, area.bottom - h - margin)
                   : area.top + margin;
  return 0;
}
static unsigned nn6_pro_overlay_window(void *owner, int x, int y,
                                       unsigned char opacity) {
  HWND window = static_cast<HWND>(owner);
  if (!IsWindow(window))
    return ERROR_INVALID_WINDOW_HANDLE;
  LONG_PTR style = GetWindowLongPtrW(window, GWL_EXSTYLE);
  style = (style & ~WS_EX_APPWINDOW) | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE |
          WS_EX_TRANSPARENT | WS_EX_LAYERED;
  SetLastError(0);
  if (!SetWindowLongPtrW(window, GWL_EXSTYLE, style) && GetLastError())
    return last_error();
  if (!SetLayeredWindowAttributes(window, 0, opacity, LWA_ALPHA))
    return last_error();
  if (!SetWindowPos(window, HWND_TOPMOST, x, y, 0, 0,
                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW |
                        SWP_FRAMECHANGED))
    return last_error();
  return 0;
}
extern "C" unsigned nn6_pro_configure_overlay(void *owner, void *main_window,
                                              int corner,
                                              unsigned char opacity) {
  RECT bounds{};
  HWND window = static_cast<HWND>(owner);
  if (!GetWindowRect(window, &bounds))
    return last_error();
  UINT dpi = GetDpiForWindow(window);
  if (!dpi)
    dpi = 96;
  unsigned width = static_cast<unsigned>(
      std::max(1, MulDiv(bounds.right - bounds.left, 96, dpi)));
  unsigned height = static_cast<unsigned>(
      std::max(1, MulDiv(bounds.bottom - bounds.top, 96, dpi)));
  int x = 0, y = 0;
  unsigned error = nn6_pro_overlay_position(
      main_window, width, height, static_cast<unsigned>(corner), &x, &y);
  return error ? error : nn6_pro_overlay_window(owner, x, y, opacity);
}
