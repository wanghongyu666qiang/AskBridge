use std::{mem::size_of, ptr};

use askbridge_core::{AppError, Result, ScreenRect};
use windows_sys::Win32::{
    Foundation::{LPARAM, RECT},
    Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO},
    UI::WindowsAndMessaging::{
        GetSystemMetrics, MONITORINFOF_PRIMARY, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
        SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    },
};

use crate::util::last_error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub bounds: ScreenRect,
    pub work_area: ScreenRect,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopLayout {
    pub virtual_bounds: ScreenRect,
    pub monitors: Vec<Monitor>,
}

impl DesktopLayout {
    pub fn enumerate() -> Result<Self> {
        let virtual_bounds = virtual_screen_bounds()?;
        let mut accumulator = MonitorAccumulator::default();
        // SAFETY: The callback is synchronous and receives a valid pointer to accumulator.
        let enumerated = unsafe {
            EnumDisplayMonitors(
                ptr::null_mut(),
                ptr::null(),
                Some(enumerate_monitor),
                (&mut accumulator as *mut MonitorAccumulator) as LPARAM,
            )
        };
        if enumerated == 0 {
            if let Some(error) = accumulator.error {
                return Err(error);
            }
            return Err(AppError::Windows {
                operation: "EnumDisplayMonitors",
                win32_code: last_error(),
            });
        }
        if accumulator.monitors.is_empty() {
            return Err(AppError::CaptureFailed(
                "Windows reported no active display monitors".to_owned(),
            ));
        }
        Ok(Self {
            virtual_bounds,
            monitors: accumulator.monitors,
        })
    }
}

#[derive(Default)]
struct MonitorAccumulator {
    monitors: Vec<Monitor>,
    error: Option<AppError>,
}

unsafe extern "system" fn enumerate_monitor(
    monitor: HMONITOR,
    _device_context: HDC,
    _monitor_rect: *mut RECT,
    data: LPARAM,
) -> i32 {
    // SAFETY: data is the MonitorAccumulator pointer supplied to EnumDisplayMonitors.
    let accumulator = unsafe { &mut *(data as *mut MonitorAccumulator) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT::default(),
        rcWork: RECT::default(),
        dwFlags: 0,
    };
    // SAFETY: monitor is supplied by Windows and info has the documented size.
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        accumulator.error = Some(AppError::Windows {
            operation: "GetMonitorInfoW",
            win32_code: last_error(),
        });
        return 0;
    }
    let Some(bounds) = screen_rect(info.rcMonitor) else {
        accumulator.error = Some(AppError::CaptureFailed(
            "Windows reported an invalid monitor rectangle".to_owned(),
        ));
        return 0;
    };
    let Some(work_area) = screen_rect(info.rcWork) else {
        accumulator.error = Some(AppError::CaptureFailed(
            "Windows reported an invalid monitor work area".to_owned(),
        ));
        return 0;
    };
    accumulator.monitors.push(Monitor {
        bounds,
        work_area,
        primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
    });
    1
}

fn virtual_screen_bounds() -> Result<ScreenRect> {
    // SAFETY: GetSystemMetrics is a read-only system query for the virtual desktop.
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    if width <= 0 || height <= 0 {
        return Err(AppError::CaptureFailed(
            "virtual desktop dimensions are invalid".to_owned(),
        ));
    }
    Ok(ScreenRect::new(left, top, width as u32, height as u32))
}

fn screen_rect(rect: RECT) -> Option<ScreenRect> {
    let width = rect.right.checked_sub(rect.left)?;
    let height = rect.bottom.checked_sub(rect.top)?;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(ScreenRect::new(
        rect.left,
        rect.top,
        width as u32,
        height as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_negative_monitor_rectangles() {
        let rect = screen_rect(RECT {
            left: -1920,
            top: -200,
            right: 0,
            bottom: 880,
        })
        .expect("valid monitor");

        assert_eq!(rect, ScreenRect::new(-1920, -200, 1920, 1080));
    }

    #[test]
    fn rejects_inverted_monitor_rectangles() {
        assert_eq!(
            screen_rect(RECT {
                left: 10,
                top: 10,
                right: 5,
                bottom: 50,
            }),
            None
        );
    }
}
