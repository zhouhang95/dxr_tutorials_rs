#![cfg_attr(debug_assertions, allow(dead_code, unused_imports, unused_variables, unused_mut))]

use windows::{
    core::*, Win32::Foundation::*, Win32::Graphics::Direct3D::Dxc::*, Win32::Graphics::Direct3D::*,
    Win32::Graphics::Direct3D12::*, Win32::Graphics::Dxgi::Common::*, Win32::{Graphics::Dxgi::*, UI::Input::KeyboardAndMouse::VK_ESCAPE},
    Win32::System::LibraryLoader::*, Win32::System::Threading::*,
    Win32::System::WindowsProgramming::*, Win32::UI::WindowsAndMessaging::*,
};

use memoffset::offset_of;

use std::mem::transmute;

fn msg_box(msg: &str) {
    let msg: HSTRING = msg.into();
    unsafe {
        MessageBoxW(None, &msg, w!("Error"), MB_OK);
    }
}

extern "system" fn wndproc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CLOSE => {
            unsafe { DestroyWindow(window) };
            LRESULT::default()
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT::default()
        }
        WM_KEYDOWN => {
            if wparam.0 == VK_ESCAPE.0 as usize {
                unsafe { PostQuitMessage(0) };
            }
            LRESULT::default()
        }
        _ => {
            unsafe { DefWindowProcA(window, message, wparam, lparam) }
        }
    }
}

fn create_window(win_title: &str, width: i32, height: i32) -> HWND {
    let class_name = w!("DxrTutorialWindowClass");

    let instance = unsafe { GetModuleHandleW(None).unwrap() };

    // Register the window class
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: instance,
        lpszClassName: class_name,
        ..Default::default()
    };

    if (unsafe { RegisterClassExW(&wc) } == 0) {
        msg_box("RegisterClass() failed");
        unreachable!()
    }

    // Window size we have is for client area, calculate actual window size
    let mut r = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    unsafe { AdjustWindowRect(&mut r, WS_OVERLAPPEDWINDOW, false) };

    let window_width = r.right - r.left;
    let window_height = r.bottom - r.top;

    // create the window
    let w_title: HSTRING = win_title.into();

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            &w_title,
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            window_width,
            window_height,
            None, // no parent window
            None, // no menus
            instance,
            None,
        )
    };

    if hwnd.0 == 0 {
        msg_box("CreateWindowEx() failed");
        unreachable!()
    }

    return hwnd;

}

fn msg_loop() {
    let mut message = MSG::default();
    loop {
        if unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.into() {
            if message.message == WM_QUIT {
                break;
            }
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        } else {

        }
    }
}


struct Tutorial {

}

impl Tutorial {
    fn new() -> Self {
        Self {

        }
    }
    fn on_load(&mut self, hwnd: HWND, width: i32, height: i32) {

    }
    fn on_frame_render(&mut self) {
        
    }
    fn on_shutdown(&mut self) {

    }
}

fn main() {
    let mut tutorial = Tutorial::new();

    let hwnd = create_window("fuck", 640, 360);

    // Calculate the client-rect area
    let mut r = RECT::default();
    unsafe { GetClientRect(hwnd, &mut r) };
    let width = r.right - r.left;
    let height = r.bottom - r.top;

    // Call onLoad()
    tutorial.on_load(hwnd, width, height);

    // Show the window
    unsafe { ShowWindow(hwnd, SW_SHOWNORMAL) };

    // Start the msgLoop()
    msg_loop();

    // Cleanup
    unsafe { DestroyWindow(hwnd) };
}
