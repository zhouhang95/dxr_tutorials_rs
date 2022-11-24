#![cfg_attr(debug_assertions, allow(dead_code, unused_imports, unused_variables, unused_mut))]

use windows::{
    core::*, Win32::Foundation::*, Win32::Graphics::Direct3D::Dxc::*, Win32::Graphics::Direct3D::*,
    Win32::Graphics::Direct3D12::*, Win32::Graphics::Dxgi::Common::*, Win32::{Graphics::Dxgi::*, UI::Input::KeyboardAndMouse::VK_ESCAPE},
    Win32::System::LibraryLoader::*, Win32::System::Threading::*,
    Win32::System::WindowsProgramming::*, Win32::UI::WindowsAndMessaging::*,
};

use memoffset::offset_of;
use glam::*;

use std::{mem::transmute, ffi::c_void};
use std::mem::size_of;

const DEFAULT_SWAP_CHAIN_BUFFERS: u32 = 3;
const RTV_HEAP_SIZE: u32 = 3;
const DEBUG_MODE: bool = true;

unsafe fn msg_box(msg: &str) {
    let msg: HSTRING = msg.into();
    MessageBoxW(None, &msg, w!("Error"), MB_OK);
}

extern "system" fn wndproc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_CLOSE => {
                DestroyWindow(window);
                LRESULT::default()
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT::default()
            }
            WM_KEYDOWN => {
                if wparam.0 == VK_ESCAPE.0 as usize {
                    PostQuitMessage(0);
                }
                LRESULT::default()
            }
            _ => {
                DefWindowProcW(window, message, wparam, lparam)
            }
        }
    }
}

unsafe fn create_window(win_title: &str, width: i32, height: i32) -> HWND {
    let class_name = w!("DxrTutorialWindowClass");

    let instance = GetModuleHandleW(None).unwrap();

    // Register the window class
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: instance,
        lpszClassName: class_name,
        ..Default::default()
    };

    if RegisterClassExW(&wc) == 0 {
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
    AdjustWindowRect(&mut r, WS_OVERLAPPEDWINDOW, false);

    let window_width = r.right - r.left;
    let window_height = r.bottom - r.top;

    // create the window
    let w_title: HSTRING = win_title.into();

    let hwnd = CreateWindowExW(
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
    );

    if hwnd.0 == 0 {
        msg_box("CreateWindowEx() failed");
        unreachable!()
    }

    return hwnd;

}

unsafe fn msg_loop(tutorial: &mut Tutorial) {
    let mut message = MSG::default();
    loop {
        if PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).into() {
            if message.message == WM_QUIT {
                break;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        } else {
            tutorial.on_frame_render();

        }
    }
}

unsafe fn create_device(factory: IDXGIFactory4) -> ID3D12Device5 {
    for i in 0.. {
        // Find the HW adapter
        let adapter = factory.EnumAdapters1(i).unwrap();
        let desc = adapter.GetDesc1().unwrap();

        // Skip SW adapters
        if (DXGI_ADAPTER_FLAG(desc.Flags) & DXGI_ADAPTER_FLAG_SOFTWARE) != DXGI_ADAPTER_FLAG_NONE {
            continue;
        }
        let mut device: Option<ID3D12Device5> = None;
        if D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_12_0, &mut device).is_ok() {
            let device = device.unwrap();
            let mut features5 = D3D12_FEATURE_DATA_D3D12_OPTIONS5::default();
            let featuresupportdatasize = size_of::<D3D12_FEATURE_DATA_D3D12_OPTIONS5>() as u32;
            device.CheckFeatureSupport(D3D12_FEATURE_D3D12_OPTIONS5, &mut features5 as *mut _ as _, featuresupportdatasize).unwrap();
            if features5.RaytracingTier != D3D12_RAYTRACING_TIER_NOT_SUPPORTED {
                return device;
            }
        }
    }
    msg_box("Raytracing is not supported on this device. Make sure your GPU supports DXR (such as Nvidia's Volta or Turing RTX) and you're on the latest drivers. The DXR fallback layer is not supported.");
    unreachable!()
}

unsafe fn create_command_queue(device: ID3D12Device5) -> ID3D12CommandQueue {
    let mut cq_desc = D3D12_COMMAND_QUEUE_DESC::default();
    cq_desc.Flags = D3D12_COMMAND_QUEUE_FLAG_NONE;
    cq_desc.Type = D3D12_COMMAND_LIST_TYPE_DIRECT;
    device.CreateCommandQueue(&cq_desc as _).unwrap()
}

unsafe fn create_dxgi_swap_chain(factory: IDXGIFactory4, hwnd: HWND, width: i32, height: i32, format: DXGI_FORMAT, command_queue: ID3D12CommandQueue) -> IDXGISwapChain3 {
    let mut swap_chain_desc = DXGI_SWAP_CHAIN_DESC1::default();
    swap_chain_desc.BufferCount = DEFAULT_SWAP_CHAIN_BUFFERS;
    swap_chain_desc.Width = width as u32;
    swap_chain_desc.Height = height as u32;
    swap_chain_desc.Format = format;
    swap_chain_desc.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
    swap_chain_desc.SwapEffect = DXGI_SWAP_EFFECT_FLIP_DISCARD;
    swap_chain_desc.SampleDesc.Count = 1;

    // CreateSwapChainForHwnd() doesn't accept IDXGISwapChain3 (Why MS? Why?)
    factory.CreateSwapChainForHwnd(&command_queue, hwnd, &swap_chain_desc, None, None)
    .unwrap()
    .cast()
    .unwrap()
}

unsafe fn create_descriptor_heap(device: ID3D12Device5, count: u32, heap_type: D3D12_DESCRIPTOR_HEAP_TYPE, shader_visible: bool) -> ID3D12DescriptorHeap {
    let mut desc = D3D12_DESCRIPTOR_HEAP_DESC::default();
    desc.NumDescriptors = count;
    desc.Type = heap_type;
    desc.Flags = if shader_visible { D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE } else { D3D12_DESCRIPTOR_HEAP_FLAG_NONE };
    device.CreateDescriptorHeap(&desc).unwrap()
}

unsafe fn create_rtv(device: ID3D12Device5, resource: &ID3D12Resource, rtv_heap: &mut HeapData, format: DXGI_FORMAT) -> D3D12_CPU_DESCRIPTOR_HANDLE {
    let mut desc = D3D12_RENDER_TARGET_VIEW_DESC::default();
    desc.ViewDimension = D3D12_RTV_DIMENSION_TEXTURE2D;
    desc.Format = format;
    desc.Anonymous.Texture2D.MipSlice = 0;
    let mut rtv_handle = rtv_heap.heap.GetCPUDescriptorHandleForHeapStart();
    rtv_handle.ptr += (rtv_heap.used_entries * device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)) as usize;
    rtv_heap.used_entries += 1;
    device.CreateRenderTargetView(resource, Some(&desc), rtv_handle);
    rtv_handle
}

unsafe fn resource_barrier(cmd_list: ID3D12GraphicsCommandList4, resource: ID3D12Resource, state_before: D3D12_RESOURCE_STATES, state_after: D3D12_RESOURCE_STATES) {
    let mut barrier = D3D12_RESOURCE_BARRIER::default();
    barrier.Type = D3D12_RESOURCE_BARRIER_TYPE_TRANSITION;
    barrier.Anonymous = D3D12_RESOURCE_BARRIER_0 {
        Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
            pResource: Some(resource.clone()),
            StateBefore: state_before,
            StateAfter: state_after,
            Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
        })
    };
    cmd_list.ResourceBarrier(&[barrier]);
}

struct Tutorial {
    hwnd: HWND,
    swap_chain_size: IVec2,
    dxgi_factory: IDXGIFactory4,
    device: ID3D12Device5,
    cmd_queue: ID3D12CommandQueue,
    swap_chain: IDXGISwapChain3,
    rtv_heap: HeapData,
    frame_objects: [FrameObject; DEFAULT_SWAP_CHAIN_BUFFERS as usize],
    cmd_list: ID3D12GraphicsCommandList4,
    fence: ID3D12Fence,
    fence_event: HANDLE,
    fence_value: u64,
}

struct HeapData {
    heap: ID3D12DescriptorHeap,
    used_entries: u32,
}
struct FrameObject {
    pub cmd_allocator: ID3D12CommandAllocator,
    pub swap_chain_buffer: ID3D12Resource,
    pub rtv_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
}

impl Tutorial {
    unsafe fn submit_cmd_list(&mut self) {
        self.cmd_list.Close().unwrap();
        let command_list = ID3D12CommandList::from(&self.cmd_list);
        self.cmd_queue.ExecuteCommandLists(&[Some(command_list)]);
        self.fence_value += 1;
        self.cmd_queue.Signal(&self.fence, self.fence_value).unwrap();
    }
    unsafe fn init_dxr(hwnd: HWND, width: i32, height: i32) -> Self {
        if DEBUG_MODE {
            let mut debug: Option<ID3D12Debug> = None;
            if let Some(debug) = D3D12GetDebugInterface(&mut debug).ok().and(debug) {
                debug.EnableDebugLayer();
            }
        }
        let dxgi_factory_flags = if cfg!(debug_assertions) { DXGI_CREATE_FACTORY_DEBUG } else { 0 };
        let dxgi_factory: IDXGIFactory4 = CreateDXGIFactory2(dxgi_factory_flags).unwrap();
        let device = create_device(dxgi_factory.clone());
        let cmd_queue = create_command_queue(device.clone());
        let swap_chain = create_dxgi_swap_chain(dxgi_factory.clone(), hwnd, width, height, DXGI_FORMAT_R8G8B8A8_UNORM, cmd_queue.clone());
        let mut rtv_heap = HeapData {
            heap: create_descriptor_heap(device.clone(), RTV_HEAP_SIZE, D3D12_DESCRIPTOR_HEAP_TYPE_RTV, false),
            used_entries: 0,
        };

        let frame_objects: [FrameObject; DEFAULT_SWAP_CHAIN_BUFFERS as usize] = array_init::array_init(|i: usize| -> FrameObject {
            let cmd_allocator: ID3D12CommandAllocator = device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT).unwrap();
            let swap_chain_buffer: ID3D12Resource = swap_chain.GetBuffer(i as u32).unwrap();
            let rtv_handle = create_rtv(device.clone(), &swap_chain_buffer, &mut rtv_heap, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB);
            FrameObject {
                cmd_allocator,
                swap_chain_buffer,
                rtv_handle,
            }
        });
        let cmd_list: ID3D12GraphicsCommandList4 = device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &frame_objects[0].cmd_allocator, None).unwrap();
        let fence: ID3D12Fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE).unwrap();
        let fence_event: HANDLE = CreateEventW(None, false, false, None).unwrap();
        Self {
            hwnd,
            swap_chain_size: ivec2(width, height),
            dxgi_factory,
            device,
            cmd_queue,
            swap_chain,
            rtv_heap,
            frame_objects,
            cmd_list,
            fence,
            fence_event,
            fence_value: 0,
        }
    }
    unsafe fn on_load(hwnd: HWND, width: i32, height: i32) -> Self {
        Self::init_dxr(hwnd, width, height)
    }
    unsafe fn begin_frame(&mut self) -> usize {
        self.swap_chain.GetCurrentBackBufferIndex() as usize
    }
    unsafe fn end_frame(&mut self, rtv_index: usize) {
        resource_barrier(self.cmd_list.clone(), self.frame_objects[rtv_index].swap_chain_buffer.clone(), D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PRESENT);
        self.submit_cmd_list();
        self.swap_chain.Present(0, 0).unwrap();

        // Prepare the command list for the next frame
        let buffer_index = self.swap_chain.GetCurrentBackBufferIndex() as usize;

        // Make sure we have the new back-buffer is ready
        if self.fence_value > DEFAULT_SWAP_CHAIN_BUFFERS as u64 {
            self.fence.SetEventOnCompletion(self.fence_value - DEFAULT_SWAP_CHAIN_BUFFERS as u64 + 1, self.fence_event).unwrap();
            WaitForSingleObject(self.fence_event, INFINITE);
        }

        self.frame_objects[buffer_index].cmd_allocator.Reset().unwrap();
        self.cmd_list.Reset(&self.frame_objects[buffer_index].cmd_allocator, None).unwrap();
    }
    unsafe fn on_frame_render(&mut self) {
        let rtv_index: usize = self.begin_frame();
        let clear_color: [f32; 4] = vec4(0.4, 0.6, 0.2, 1.0).into();
        resource_barrier(self.cmd_list.clone(), self.frame_objects[rtv_index].swap_chain_buffer.clone(), D3D12_RESOURCE_STATE_PRESENT, D3D12_RESOURCE_STATE_RENDER_TARGET);
        self.cmd_list.ClearRenderTargetView(self.frame_objects[rtv_index].rtv_handle, clear_color.as_ptr(), &[]);
        self.end_frame(rtv_index);
    }
    unsafe fn on_shutdown(&mut self) {
        // Wait for the command queue to finish execution
        self.fence_value += 1;
        self.cmd_queue.Signal(&self.fence, self.fence_value).unwrap();
        self.fence.SetEventOnCompletion(self.fence_value, self.fence_event).unwrap();
        WaitForSingleObject(self.fence_event, INFINITE);
    }
}

unsafe fn unsafe_main() {
    let hwnd = create_window("fuck", 640, 360);

    // Calculate the client-rect area
    let mut r = RECT::default();
    GetClientRect(hwnd, &mut r);
    let width = r.right - r.left;
    let height = r.bottom - r.top;

    // Call onLoad()
    let mut tutorial = Tutorial::on_load(hwnd, width, height);

    // Show the window
    ShowWindow(hwnd, SW_SHOWNORMAL);

    // Start the msgLoop()
    msg_loop(&mut tutorial);

    // Cleanup
    DestroyWindow(hwnd);
}

fn main() {
    unsafe { unsafe_main() };
}
