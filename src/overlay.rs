//! Small always-on-top window showing the current match (`--overlay`, Windows only).
//!
//! It is meant to be readable at a glance on the accept/decline screen: where
//! the match goes and how far it is. The window never takes the focus away
//! from the game, can be dragged with the mouse, and is updated from the
//! monitor through [`Overlay::show`].

/// What the overlay displays.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Waiting,
    Match {
        p2p: bool,
        /// Cloud provider and region (client-server), empty for peer-to-peer.
        provider: String,
        /// Country-Region-City, or the cloud region when unknown.
        area: String,
        /// Latest latency in ms (initial latency until the first sample).
        latency: i64,
        loss: f32,
    },
}

/// Text lines shown by the window (also used by tests).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn lines(s: &Status) -> [String; 3] {
    match s {
        Status::Waiting => [
            "efootball-monitor".into(),
            "Waiting for a match...".into(),
            String::new(),
        ],
        Status::Match {
            p2p,
            provider,
            area,
            latency,
            loss,
        } => {
            let head = if *p2p {
                "PEER-TO-PEER".to_string()
            } else if provider.is_empty() {
                "CLIENT-SERVER".to_string()
            } else {
                format!("CLIENT-SERVER  ·  {provider}")
            };
            let lat = if *latency > 0 {
                format!("{latency} ms")
            } else {
                "-- ms".into()
            };
            [head, area.clone(), format!("{lat}    {loss:.1}% loss")]
        }
    }
}

/// Latency color as 0xRRGGBB: green, yellow, orange, red.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn latency_rgb(ms: i64) -> u32 {
    match ms {
        i64::MIN..=0 => 0xb0b0b0,
        1..=59 => 0x4ade80,
        60..=99 => 0xfacc15,
        100..=149 => 0xfb923c,
        _ => 0xf87171,
    }
}

pub struct Overlay {
    #[cfg(windows)]
    _private: (),
}

impl Overlay {
    /// Opens the window on its own thread. Returns `None` where unsupported.
    pub fn start() -> Option<Overlay> {
        #[cfg(windows)]
        {
            win::start();
            Some(Overlay { _private: () })
        }
        #[cfg(not(windows))]
        {
            crate::util::out("The overlay window is only available on Windows.\n");
            None
        }
    }

    pub fn show(&self, status: Status) {
        #[cfg(windows)]
        win::update(status);
        #[cfg(not(windows))]
        let _ = status;
    }
}

#[cfg(windows)]
mod win {
    use super::{latency_rgb, lines, Status};
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    const WIDTH: i32 = 380;
    const HEIGHT: i32 = 92;
    const WM_STATUS: u32 = WM_APP + 1;

    static HWND_: AtomicIsize = AtomicIsize::new(0);
    static STATUS: Mutex<Option<Status>> = Mutex::new(None);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    /// 0xRRGGBB → COLORREF (0x00BBGGRR).
    fn colorref(rgb: u32) -> u32 {
        ((rgb & 0xff) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 0xff)
    }

    pub fn update(status: Status) {
        let mut cur = STATUS.lock().unwrap();
        if cur.as_ref() == Some(&status) {
            return;
        }
        *cur = Some(status);
        drop(cur);
        let hwnd = HWND_.load(Ordering::Acquire);
        if hwnd != 0 {
            // SAFETY: posting to a window owned by another thread is allowed.
            unsafe { PostMessageW(hwnd as HWND, WM_STATUS, 0, 0) };
        }
    }

    pub fn start() {
        STATUS.lock().unwrap().get_or_insert(Status::Waiting);
        std::thread::spawn(|| {
            // SAFETY: plain Win32 window creation and message loop, all on this thread.
            unsafe { run() }
        });
    }

    unsafe fn run() {
        let class = wide("EfmOverlay");
        let hinst = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: 0 as _,
            hCursor: LoadCursorW(0 as _, IDC_SIZEALL),
            hbrBackground: 0 as _,
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let title = wide("efootball-monitor");
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            screen_w - WIDTH - 24,
            24,
            WIDTH,
            HEIGHT,
            0 as _,
            0 as _,
            hinst,
            std::ptr::null(),
        );
        if hwnd == 0 as _ {
            crate::util::out("Could not open the overlay window.\n");
            return;
        }
        SetLayeredWindowAttributes(hwnd, 0, 230, LWA_ALPHA);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        HWND_.store(hwnd as isize, Ordering::Release);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, 0 as _, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        HWND_.store(0, Ordering::Release);
    }

    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            // Drag the window from anywhere.
            WM_NCHITTEST => HTCAPTION as LRESULT,
            // Never steal the focus from the game.
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            // Right click hides the window until the next match.
            WM_NCRBUTTONUP => {
                ShowWindow(hwnd, SW_HIDE);
                0
            }
            WM_STATUS => {
                if matches!(*STATUS.lock().unwrap(), Some(Status::Match { .. })) {
                    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
                InvalidateRect(hwnd, std::ptr::null(), 1);
                0
            }
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                paint(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    unsafe fn font(height: i32, weight: i32) -> HFONT {
        let face = wide("Segoe UI");
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as _,
            OUT_DEFAULT_PRECIS as _,
            CLIP_DEFAULT_PRECIS as _,
            CLEARTYPE_QUALITY as _,
            (DEFAULT_PITCH | FF_DONTCARE) as _,
            face.as_ptr(),
        )
    }

    unsafe fn text(hdc: HDC, x: i32, y: i32, s: &str, f: HFONT, rgb: u32) {
        let w: Vec<u16> = s.encode_utf16().collect();
        let old = SelectObject(hdc, f as _);
        SetTextColor(hdc, colorref(rgb));
        TextOutW(hdc, x, y, w.as_ptr(), w.len() as i32);
        SelectObject(hdc, old);
    }

    unsafe fn paint(hwnd: HWND) {
        let status = STATUS.lock().unwrap().clone().unwrap_or(Status::Waiting);
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let rect = RECT {
            left: 0,
            top: 0,
            right: WIDTH,
            bottom: HEIGHT,
        };
        let bg = CreateSolidBrush(colorref(0x111827));
        FillRect(hdc, &rect, bg);
        DeleteObject(bg as _);

        let (accent, latency) = match &status {
            Status::Match { latency, .. } => (latency_rgb(*latency), *latency),
            Status::Waiting => (0x4b5563, 0),
        };
        let bar = CreateSolidBrush(colorref(accent));
        let left = RECT {
            left: 0,
            top: 0,
            right: 6,
            bottom: HEIGHT,
        };
        FillRect(hdc, &left, bar);
        DeleteObject(bar as _);

        SetBkMode(hdc, TRANSPARENT as _);
        let small = font(16, FW_SEMIBOLD as i32);
        let big = font(26, FW_BOLD as i32);
        let [head, area, stats] = lines(&status);
        text(hdc, 18, 8, &head, small, 0x9ca3af);
        text(hdc, 18, 28, &area, big, 0xf9fafb);
        if !stats.is_empty() {
            let c = if latency > 0 { accent } else { 0x9ca3af };
            text(hdc, 18, 62, &stats, small, c);
        }
        DeleteObject(small as _);
        DeleteObject(big as _);
        EndPaint(hwnd, &ps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_lines() {
        let s = Status::Match {
            p2p: false,
            provider: "Google Cloud asia-northeast1".into(),
            area: "Japan-Tokyo-Tokyo".into(),
            latency: 182,
            loss: 2.5,
        };
        assert_eq!(
            lines(&s),
            [
                "CLIENT-SERVER  ·  Google Cloud asia-northeast1".to_string(),
                "Japan-Tokyo-Tokyo".to_string(),
                "182 ms    2.5% loss".to_string()
            ]
        );
        assert_eq!(latency_rgb(182), 0xf87171);
        assert_eq!(latency_rgb(35), 0x4ade80);
    }
}
