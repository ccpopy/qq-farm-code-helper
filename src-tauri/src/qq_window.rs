#[cfg(windows)]
use std::{collections::BTreeSet, path::Path};

#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND, LPARAM},
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                QueryFullProcessImageNameW,
            },
        },
        UI::{
            Accessibility::{
                CUIAutomation, IUIAutomation, TreeScope_Descendants, UIA_ButtonControlTypeId,
            },
            WindowsAndMessaging::{
                EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
            },
        },
    },
    core::{BOOL, PWSTR},
};

pub fn visible_nicknames() -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        std::thread::spawn(read_visible_nicknames)
            .join()
            .map_err(|_| "读取 QQ 主界面时发生异常，本次不会自动绑定 QQ 号".to_owned())?
    }

    #[cfg(not(windows))]
    {
        Err("当前系统不支持读取 Windows QQ 主界面".to_owned())
    }
}

#[cfg(windows)]
fn read_visible_nicknames() -> Result<Vec<String>, String> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|error| format!("初始化 Windows 界面检测失败: {error}"))?;
    }
    let _com = ComGuard;

    let automation: IUIAutomation = unsafe {
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            .map_err(|error| format!("启动 Windows 界面检测失败: {error}"))?
    };
    let mut nicknames = BTreeSet::new();
    for window in qq_windows()? {
        if let Ok(nickname) = nickname_from_window(&automation, window) {
            nicknames.insert(nickname);
        }
    }
    if nicknames.is_empty() {
        return Err("无法从 QQ 主界面读取当前昵称，请保持新版 QQ 主窗口已打开".to_owned());
    }
    Ok(nicknames.into_iter().collect())
}

#[cfg(windows)]
fn nickname_from_window(automation: &IUIAutomation, window: HWND) -> Result<String, String> {
    let root = unsafe { automation.ElementFromHandle(window) }
        .map_err(|error| format!("无法读取 QQ 主窗口: {error}"))?;
    let root_bounds = unsafe { root.CurrentBoundingRectangle() }.ok();
    let condition = unsafe { automation.CreateTrueCondition() }
        .map_err(|error| format!("创建 QQ 界面检测条件失败: {error}"))?;
    let elements = unsafe { root.FindAll(TreeScope_Descendants, &condition) }
        .map_err(|error| format!("遍历 QQ 主界面失败: {error}"))?;
    let count = unsafe { elements.Length() }
        .map_err(|error| format!("读取 QQ 界面元素数量失败: {error}"))?;

    let mut header_names = BTreeSet::new();
    let mut all_names = BTreeSet::new();
    for index in 0..count {
        let Ok(element) = (unsafe { elements.GetElement(index) }) else {
            continue;
        };
        if unsafe { element.CurrentControlType() }.ok() != Some(UIA_ButtonControlTypeId) {
            continue;
        }
        let Ok(name) = (unsafe { element.CurrentName() }) else {
            continue;
        };
        let name = name.to_string();
        let Some(nickname) = name.trim().strip_suffix("的头像") else {
            continue;
        };
        let nickname = nickname.trim();
        if nickname.is_empty() {
            continue;
        }
        all_names.insert(nickname.to_owned());
        if is_header_element(
            root_bounds,
            unsafe { element.CurrentBoundingRectangle() }.ok(),
        ) {
            header_names.insert(nickname.to_owned());
        }
    }

    unique_nickname(&header_names)
        .or_else(|| unique_nickname(&all_names))
        .ok_or_else(|| {
            if all_names.len() > 1 {
                "QQ 主界面出现多个头像昵称，无法唯一确认当前账号".to_owned()
            } else {
                "无法从 QQ 主界面读取当前昵称，请保持新版 QQ 主窗口已打开".to_owned()
            }
        })
}

#[cfg(windows)]
fn qq_windows() -> Result<Vec<HWND>, String> {
    let mut windows = Vec::new();
    unsafe {
        EnumWindows(
            Some(collect_qq_windows),
            LPARAM((&raw mut windows).cast::<()>() as isize),
        )
    }
    .map_err(|error| format!("枚举 Windows 窗口失败: {error}"))?;
    if windows.is_empty() {
        return Err("未找到 QQ 主窗口，请先打开新版 Windows QQ 主界面".to_owned());
    }
    Ok(windows)
}

#[cfg(windows)]
unsafe extern "system" fn collect_qq_windows(window: HWND, state: LPARAM) -> BOOL {
    let windows = unsafe { &mut *(state.0 as *mut Vec<HWND>) };
    if is_qq_process(window)
        && window_title(window).is_ok_and(|title| is_qq_main_window_title(&title))
    {
        windows.push(window);
    }
    BOOL(1)
}

#[cfg(windows)]
fn window_title(window: HWND) -> Result<String, String> {
    let length = unsafe { GetWindowTextLengthW(window) };
    if length <= 0 {
        return Err("窗口没有可读取的标题".to_owned());
    }
    let mut buffer = vec![0_u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(window, &mut buffer) };
    if copied <= 0 {
        return Err("读取窗口标题失败".to_owned());
    }
    Ok(String::from_utf16_lossy(&buffer[..copied as usize]))
}

#[cfg(windows)]
fn is_qq_main_window_title(title: &str) -> bool {
    title.trim() == "QQ"
}

#[cfg(windows)]
fn is_header_element(
    root: Option<windows::Win32::Foundation::RECT>,
    element: Option<windows::Win32::Foundation::RECT>,
) -> bool {
    let (Some(root), Some(element)) = (root, element) else {
        return false;
    };
    element.left >= root.left
        && element.top >= root.top
        && element.left < root.left.saturating_add(320)
        && element.top < root.top.saturating_add(160)
}

#[cfg(windows)]
fn unique_nickname(values: &BTreeSet<String>) -> Option<String> {
    (values.len() == 1)
        .then(|| values.first().cloned())
        .flatten()
}

#[cfg(windows)]
fn is_qq_process(window: HWND) -> bool {
    process_executable(window).is_ok_and(|executable| {
        Path::new(&executable)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("QQ.exe"))
    })
}

#[cfg(windows)]
fn process_executable(window: HWND) -> Result<String, String> {
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
    if process_id == 0 {
        return Err("无法确认 QQ 主窗口所属进程".to_owned());
    }

    let process = unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)
            .map_err(|error| format!("无法读取 QQ 进程信息: {error}"))?
    };
    let _process = HandleGuard(process);
    let mut buffer = vec![0_u16; 32_768];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(|error| format!("无法确认 QQ 程序路径: {error}"))?;
    Ok(String::from_utf16_lossy(&buffer[..length as usize]))
}

#[cfg(windows)]
struct ComGuard;

#[cfg(windows)]
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

#[cfg(windows)]
struct HandleGuard(HANDLE);

#[cfg(windows)]
impl Drop for HandleGuard {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_qq_main_window_title() {
        assert!(is_qq_main_window_title("QQ"));
        assert!(is_qq_main_window_title(" QQ "));
        assert!(!is_qq_main_window_title("QQ经典农场"));
        assert!(!is_qq_main_window_title("QQ音乐"));
    }

    #[test]
    #[ignore = "requires a running Windows QQ main window"]
    fn reads_visible_qq_nicknames() {
        let nicknames = visible_nicknames().unwrap();
        eprintln!("visible QQ account count: {}", nicknames.len());
        assert!(!nicknames.is_empty());
    }
}
