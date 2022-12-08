#![cfg_attr(debug_assertions, allow(dead_code, unused_imports, unused_variables, unused_mut))]

use windows::{
    core::*, Win32::Foundation::*, Win32::Graphics::Direct3D::Dxc::*, Win32::Graphics::Direct3D::*,
    Win32::Graphics::Direct3D12::*, Win32::Graphics::Dxgi::Common::*, Win32::{Graphics::Dxgi::*, UI::Input::KeyboardAndMouse::VK_ESCAPE},
    Win32::System::LibraryLoader::*, Win32::System::Threading::*,
    Win32::System::WindowsProgramming::*, Win32::UI::WindowsAndMessaging::*,
};

use memoffset::offset_of;
use glam::*;
use once_cell::sync::Lazy;

use std::mem::{size_of, size_of_val, ManuallyDrop};
use std::ffi::c_void;

const DEFAULT_SWAP_CHAIN_BUFFERS: u32 = 3;
const RTV_HEAP_SIZE: u32 = 3;
const SRV_UAV_HEAP_SIZE: u32 = 2;

const DEBUG_MODE: bool = true;

const RAY_GEN_SHADER: &str = "rayGen";
const MISS_SHADER: &str = "miss";
const CLOSEST_HIT_SHADER: &str = "chs";
const HIT_GROUP: &str = "HitGroup";

const W_RAY_GEN_SHADER: PCWSTR = w!("rayGen");
const W_MISS_SHADER: PCWSTR = w!("miss");
const W_HIT_GROUP: PCWSTR = w!("HitGroup");
const W_CLOSEST_HIT_SHADER: PCWSTR = w!("chs");

const DXC: Lazy<D3D12ShaderCompilerInfo> = Lazy::new(|| {
    D3D12ShaderCompilerInfo::new()
});


unsafe fn memcpy<T, U>(dst: *mut T, src: *const U, count: usize) {
    std::ptr::copy_nonoverlapping::<u8>(
        src as *const _,
        dst as *mut _,
        count,
    );
}

fn align_to(alignment: u32, val: u32) -> u32 {
    ((val + alignment - 1) / alignment) * alignment
}

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

unsafe fn create_descriptor_heap(device: &ID3D12Device5, count: u32, heap_type: D3D12_DESCRIPTOR_HEAP_TYPE, shader_visible: bool) -> ID3D12DescriptorHeap {
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

const UPLOAD_HEAP_PROPS: D3D12_HEAP_PROPERTIES  = D3D12_HEAP_PROPERTIES {
    Type: D3D12_HEAP_TYPE_UPLOAD,
    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
    CreationNodeMask: 0,
    VisibleNodeMask: 0,
};

const DEFAULT_HEAP_PROPS: D3D12_HEAP_PROPERTIES = D3D12_HEAP_PROPERTIES {
    Type: D3D12_HEAP_TYPE_DEFAULT,
    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
    CreationNodeMask: 0,
    VisibleNodeMask: 0,
};

struct BLASBuffers {
    scratch: ID3D12Resource,
    result: ID3D12Resource,
}
struct TLASBuffers {
    scratch: ID3D12Resource,
    result: ID3D12Resource,
    instance_desc: ID3D12Resource,
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
    vert_buf: Option<ID3D12Resource>,
    tlas: Option<ID3D12Resource>,
    blas: Option<ID3D12Resource>,
    tlas_size: u64,
    pipeline_state: Option<ID3D12StateObject>,
    empty_root_sig: Option<ID3D12RootSignature>,
    shader_table: Option<ID3D12Resource>,
    shader_table_entry_size: u32,
    output_resource: Option<ID3D12Resource>,
    srv_uav_heap: Option<ID3D12DescriptorHeap>,
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

struct PipelineConfig {
    config: D3D12_RAYTRACING_PIPELINE_CONFIG,
    subobject: D3D12_STATE_SUBOBJECT,
}

impl PipelineConfig {
    unsafe fn new() -> Self {
        Self {
            config: std::mem::zeroed(),
            subobject: std::mem::zeroed(),
        }
    }
    unsafe fn init(&mut self, max_trace_recursion_depth: u32) {
        self.config = D3D12_RAYTRACING_PIPELINE_CONFIG { MaxTraceRecursionDepth: max_trace_recursion_depth };
        self.subobject = D3D12_STATE_SUBOBJECT {
            Type: D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_PIPELINE_CONFIG,
            pDesc: &self.config as *const _ as _,
        };
    }
}
struct ShaderConfig {
    shader_config: D3D12_RAYTRACING_SHADER_CONFIG,
    subobject: D3D12_STATE_SUBOBJECT,
}

impl ShaderConfig {
    unsafe fn new() -> Self {
        Self {
            shader_config: std::mem::zeroed(),
            subobject: std::mem::zeroed(),
        }
    }
    unsafe fn init(&mut self, max_attribute_size_in_bytes: u32, max_payload_size_in_bytes: u32) {
        self.shader_config = D3D12_RAYTRACING_SHADER_CONFIG {
            MaxPayloadSizeInBytes: max_payload_size_in_bytes,
            MaxAttributeSizeInBytes: max_attribute_size_in_bytes,
        };
        self.subobject = D3D12_STATE_SUBOBJECT {
            Type: D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_SHADER_CONFIG,
            pDesc: &self.shader_config as *const _ as _,
        };
    }
}

struct ExportAssociation {
    subobject: D3D12_STATE_SUBOBJECT,
    association: D3D12_SUBOBJECT_TO_EXPORTS_ASSOCIATION,
    export_names: Vec<HSTRING>,
    export_name_ptrs: Vec<PWSTR>,
}

impl ExportAssociation {
    unsafe fn new() -> Self {
        Self {
            subobject: std::mem::zeroed(),
            association: std::mem::zeroed(),
            export_names: Vec::new(),
            export_name_ptrs: Vec::new(),
        }
    }
    unsafe fn init(&mut self, export_names: &[String], subobject_to_associate: *const D3D12_STATE_SUBOBJECT) {
        self.export_names = export_names.iter().map( |s| s.into()).collect();
        for n in &self.export_names {
            let n = PCWSTR::from(n);
            let n: PWSTR = std::mem::transmute(n);
            self.export_name_ptrs.push(n);
        }

        self.association.NumExports = export_names.len() as _;
        self.association.pExports = self.export_name_ptrs.as_mut_ptr();
        self.association.pSubobjectToAssociate = subobject_to_associate;

        self.subobject.Type = D3D12_STATE_SUBOBJECT_TYPE_SUBOBJECT_TO_EXPORTS_ASSOCIATION;
        self.subobject.pDesc = &self.association as *const _ as _;
    }
}

struct RootSignatureDesc {
    desc: D3D12_ROOT_SIGNATURE_DESC,
    range: Vec<D3D12_DESCRIPTOR_RANGE>,
    root_params: Vec<D3D12_ROOT_PARAMETER>,
}

impl RootSignatureDesc {
    unsafe fn new() -> Self {
        Self {
            desc: std::mem::zeroed(),
            range: Vec::new(),
            root_params: Vec::new(),
        }
    }
    fn ray_gen_root_signature_desc(&mut self) {
        // gOutput
        self.range.push(D3D12_DESCRIPTOR_RANGE {
            RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_UAV,
            NumDescriptors: 1,
            BaseShaderRegister: 0,
            RegisterSpace: 0,
            OffsetInDescriptorsFromTableStart: 0,
        });

        // gRtScene
        self.range.push(D3D12_DESCRIPTOR_RANGE {
            RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
            NumDescriptors: 1,
            BaseShaderRegister: 0,
            RegisterSpace: 0,
            OffsetInDescriptorsFromTableStart: 1,
        });

        // Create the desc
        self.root_params.push(D3D12_ROOT_PARAMETER{
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
            Anonymous: D3D12_ROOT_PARAMETER_0 {
                DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                    NumDescriptorRanges: 2,
                    pDescriptorRanges: self.range.as_ptr(),
                },
            },
            ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
        });

        self.desc = D3D12_ROOT_SIGNATURE_DESC {
            NumParameters: 1,
            pParameters: self.root_params.as_ptr(),
            Flags: D3D12_ROOT_SIGNATURE_FLAG_LOCAL_ROOT_SIGNATURE,
            ..Default::default()
        };
    }

}


unsafe fn create_root_signature(device: &ID3D12Device5, desc: &D3D12_ROOT_SIGNATURE_DESC) -> ID3D12RootSignature {
    let mut sig_blob = None;
    D3D12SerializeRootSignature(
        desc as _,
        D3D_ROOT_SIGNATURE_VERSION_1,
        &mut sig_blob,
        None
    ).unwrap();
    let sig_blob = sig_blob.unwrap();

    let root_sig = device.CreateRootSignature(
        0,
        std::slice::from_raw_parts(
            sig_blob.GetBufferPointer() as _,
            sig_blob.GetBufferSize(),
        ),
    ).unwrap();
    root_sig
}

struct RootSignature {
    root_sig: ID3D12RootSignature,
    interface: *mut c_void,
    subobject: D3D12_STATE_SUBOBJECT,
}

impl RootSignature {
    unsafe fn new(device: &ID3D12Device5, desc: &D3D12_ROOT_SIGNATURE_DESC) -> Self {
        let root_sig = create_root_signature(device, desc);
        Self {
            root_sig,
            interface: std::ptr::null_mut(),
            subobject: std::mem::zeroed(),
        }
    }
    fn init(&mut self, type_: D3D12_STATE_SUBOBJECT_TYPE) {
        self.interface = self.root_sig.as_raw();
        self.subobject.pDesc = &self.interface as *const *mut _ as _;
        self.subobject.Type = type_;
    }
    fn init_local(&mut self) {
        self.init(D3D12_STATE_SUBOBJECT_TYPE_LOCAL_ROOT_SIGNATURE);
    }
    fn init_global(&mut self) {
        self.init(D3D12_STATE_SUBOBJECT_TYPE_GLOBAL_ROOT_SIGNATURE);
    }
}

struct HitProgram {
    export_name: HSTRING,
    desc: D3D12_HIT_GROUP_DESC,
    subobject: D3D12_STATE_SUBOBJECT,
}

impl HitProgram {
    unsafe fn new(name: &str) -> Self {
        Self {
            export_name: name.into(),
            desc: std::mem::zeroed(),
            subobject: std::mem::zeroed(),
        }
    }
    unsafe fn init(&mut self, ahs_export: PCWSTR, chs_export: PCWSTR) {
        self.desc = D3D12_HIT_GROUP_DESC {
            HitGroupExport: (&self.export_name).into(),
            AnyHitShaderImport: ahs_export,
            ClosestHitShaderImport: chs_export,
            ..Default::default()
        };
        self.subobject = D3D12_STATE_SUBOBJECT {
            Type: D3D12_STATE_SUBOBJECT_TYPE_HIT_GROUP,
            pDesc: &self.desc as *const _ as _,
        };
    }
}

struct D3D12ShaderCompilerInfo {
    pub library: IDxcLibrary,
    pub compiler: IDxcCompiler,
}

impl D3D12ShaderCompilerInfo {
    fn new() -> Self {
        Self {
            library: unsafe { DxcCreateInstance(&CLSID_DxcLibrary).unwrap() },
            compiler: unsafe { DxcCreateInstance(&CLSID_DxcCompiler).unwrap() },
        }
    }

    fn compile_shader_file(&self, path: &str, entry_point: &str, target_profile: &str) -> IDxcBlob {
        let path: HSTRING = path.into();
        let entry_point: HSTRING = entry_point.into();
        let target_profile: HSTRING = target_profile.into();
        unsafe {
            let source_blob = self.library.CreateBlobFromFile(&path, Some(&DXC_CP_UTF8)).unwrap();
            self.compiler.Compile(
                &source_blob,
                &path,
                &entry_point,
                &target_profile,
                None,
                &[],
                None
            ).unwrap().GetResult().unwrap()
        }
    }
}
struct DxilLibrary {
    dxil_lib_desc: D3D12_DXIL_LIBRARY_DESC,
    state_subobject: D3D12_STATE_SUBOBJECT,
    shader_blob: Option<IDxcBlob>,
    export_desc: Vec<D3D12_EXPORT_DESC>,
    export_name: Vec<HSTRING>,
}

impl DxilLibrary {
    unsafe fn new() -> Self {
        Self {
            dxil_lib_desc: std::mem::zeroed(),
            state_subobject: std::mem::zeroed(),
            shader_blob: None,
            export_desc: Vec::new(),
            export_name: Vec::new(),
        }
    }
    unsafe fn init(&mut self, shader_blob: IDxcBlob, export_point: &Vec<String>) {
        self.shader_blob = Some(shader_blob);
        self.state_subobject = D3D12_STATE_SUBOBJECT {
            Type: D3D12_STATE_SUBOBJECT_TYPE_DXIL_LIBRARY,
            pDesc: &self.dxil_lib_desc as *const _ as _,
        };

        self.export_name = export_point.iter().map( |s| s.into()).collect();
        for name in &self.export_name {
            self.export_desc.push(D3D12_EXPORT_DESC {
                Name: name.into(),
                Flags: D3D12_EXPORT_FLAG_NONE,
                ..Default::default()
            });
        }

        self.dxil_lib_desc = D3D12_DXIL_LIBRARY_DESC {
            DXILLibrary: D3D12_SHADER_BYTECODE {
                pShaderBytecode: self.shader_blob.as_ref().unwrap().GetBufferPointer(),
                BytecodeLength: self.shader_blob.as_ref().unwrap().GetBufferSize(),
            },
            NumExports: export_point.len() as _,
            pExports: self.export_desc.as_mut_ptr(),
        };
    }

    unsafe fn create_dxil_library(&mut self) {
        // Compile the shader
        let dxil_lib = DXC.compile_shader_file("res/shaders.hlsl", "", "lib_6_3");
        self.init(dxil_lib, &vec![
            RAY_GEN_SHADER.into(),
            MISS_SHADER.into(),
            CLOSEST_HIT_SHADER.into(),
        ]);
    }

}

impl Tutorial {
    unsafe fn resource_barrier(&self, resource: ID3D12Resource, state_before: D3D12_RESOURCE_STATES, state_after: D3D12_RESOURCE_STATES) {
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
        self.cmd_list.ResourceBarrier(&[barrier]);
    }
    unsafe fn create_shader_table(&mut self) {
        /* The shader-table layout is as follows:
            Entry 0 - Ray-gen program
            Entry 1 - Miss program
            Entry 2 - Hit program
            All entries in the shader-table must have the same size, so we will choose it base on the largest required entry.
            The ray-gen program requires the largest entry - sizeof(program identifier) + 8 bytes for a descriptor-table.
            The entry size must be aligned up to D3D12_RAYTRACING_SHADER_RECORD_BYTE_ALIGNMENT
        */

        // Calculate the size and create the buffer
        self.shader_table_entry_size = D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES;
        self.shader_table_entry_size += 8; // The ray-gen's descriptor table

        self.shader_table_entry_size = align_to(D3D12_RAYTRACING_SHADER_RECORD_BYTE_ALIGNMENT, self.shader_table_entry_size);
        let shader_table_size = self.shader_table_entry_size * 3;

        // For simplicity, we create the shader-table on the upload heap. You can also create it on the default heap
        let shader_table = self.create_buffer(shader_table_size as u64, D3D12_RESOURCE_FLAG_NONE, D3D12_RESOURCE_STATE_GENERIC_READ, &UPLOAD_HEAP_PROPS);

        // Map the buffer
        let mut data: *mut u8 = std::ptr::null_mut();
        shader_table.Map(0, None, Some(&mut data as *mut *mut u8 as _)).unwrap();

        let rtso_prop: ID3D12StateObjectProperties = self.pipeline_state.as_ref().unwrap().cast().unwrap();

        // Entry 0 - ray-gen program ID and descriptor data
        memcpy(data, rtso_prop.GetShaderIdentifier(W_RAY_GEN_SHADER), D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as _);

        // This is where we need to set the descriptor data for the ray-gen shader.
        let heap_start = self.srv_uav_heap.as_ref().unwrap().GetGPUDescriptorHandleForHeapStart().ptr;
        let ptr = data.offset(D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as _) as *mut u64;
        *ptr = heap_start;

        // Entry 1 - miss program
        memcpy(data.offset(self.shader_table_entry_size as _), rtso_prop.GetShaderIdentifier(W_MISS_SHADER), D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as _);

        // Entry 2 - hit program
        let mut hit_entry = data.offset((self.shader_table_entry_size * 2) as _); // +2 skips the ray-gen and miss entries
        memcpy(hit_entry, rtso_prop.GetShaderIdentifier(W_HIT_GROUP), D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as _);

        // Unmap
        shader_table.Unmap(0, None);

        // move
        self.shader_table = Some(shader_table);

    }
    unsafe fn create_rt_pipeline_state(&mut self) {
        // Need 10 subobjects:
        //  1 for the DXIL library
        //  1 for hit-group
        //  2 for RayGen root-signature (root-signature and the subobject association)
        //  2 for the root-signature shared between miss and hit shaders (signature and association)
        //  2 for shader config (shared between all programs. 1 for the config, 1 for association)
        //  1 for pipeline config
        //  1 for the global root signature
        let mut subobjects: Vec<D3D12_STATE_SUBOBJECT> = Vec::with_capacity(64);

        // Create the DXIL library
        let mut dxil_lib = DxilLibrary::new();
        dxil_lib.create_dxil_library();
        subobjects.push(dxil_lib.state_subobject); // 0 Library

        let mut hit_program = HitProgram::new(HIT_GROUP);
        hit_program.init(PCWSTR::null(), W_CLOSEST_HIT_SHADER);
        subobjects.push(hit_program.subobject); // 1 Library

        // Create the ray-gen root-signature and association
        let empty_desc = D3D12_ROOT_SIGNATURE_DESC {
            Flags: D3D12_ROOT_SIGNATURE_FLAG_LOCAL_ROOT_SIGNATURE,
            ..Default::default()
        };
        let mut ray_gen_root_signature_desc = RootSignatureDesc::new();
        ray_gen_root_signature_desc.ray_gen_root_signature_desc();
        let mut rgs_root_signature = RootSignature::new(&self.device, &ray_gen_root_signature_desc.desc);
        rgs_root_signature.init_local();
        subobjects.push(rgs_root_signature.subobject); // 2 RayGen Root Sig

        let mut rgs_root_association = ExportAssociation::new();
        rgs_root_association.init(&[RAY_GEN_SHADER.into()], &subobjects[subobjects.len() - 1]);
        subobjects.push(rgs_root_association.subobject); // 3 Associate Root Sig to RGS

        // Create the miss- and hit-programs root-signature and association
        let empty_desc = D3D12_ROOT_SIGNATURE_DESC {
            Flags: D3D12_ROOT_SIGNATURE_FLAG_LOCAL_ROOT_SIGNATURE,
            ..Default::default()
        };
        let mut hit_miss_root_signature = RootSignature::new(&self.device, &empty_desc);
        hit_miss_root_signature.init_local();
        subobjects.push(hit_miss_root_signature.subobject); // 4 Root Sig to be shared between Miss and CHS

        let mut hit_miss_root_association = ExportAssociation::new();
        hit_miss_root_association.init(&[MISS_SHADER.into(), CLOSEST_HIT_SHADER.into()], &subobjects[subobjects.len() - 1]);
        subobjects.push(hit_miss_root_association.subobject); // 5 Associate Root Sig to Miss and CHS

        // Bind the payload size to the programs
        let mut shader_config = ShaderConfig::new();
        shader_config.init((size_of::<f32>() * 2) as _, (size_of::<f32>() * 3) as _);
        subobjects.push(shader_config.subobject); // 6 Shader Config

        let mut config_association = ExportAssociation::new();
        config_association.init(&[MISS_SHADER.into(), CLOSEST_HIT_SHADER.into(), RAY_GEN_SHADER.into()], &subobjects[subobjects.len() - 1]);
        subobjects.push(config_association.subobject); // 7 Associate Shader Config to Miss, CHS, RGS

        // Create the pipeline config
        let mut config = PipelineConfig::new();
        config.init(1);
        subobjects.push(config.subobject);  // 8

        // Create the global root signature and store the empty signature
        let global_desc = D3D12_ROOT_SIGNATURE_DESC::default();
        let mut root = RootSignature::new(&self.device, &global_desc);
        root.init_global();
        self.empty_root_sig = Some(root.root_sig.clone());
        subobjects.push(root.subobject); // 9

        // Create the state
        let desc = D3D12_STATE_OBJECT_DESC {
            Type: D3D12_STATE_OBJECT_TYPE_RAYTRACING_PIPELINE,
            NumSubobjects: subobjects.len() as _,
            pSubobjects: subobjects.as_ptr(),
        };

        self.pipeline_state = Some(self.device.CreateStateObject(&desc).unwrap());
    }
    unsafe fn create_buffer(
        &self,
        size: u64,
        flags: D3D12_RESOURCE_FLAGS,
        init_state: D3D12_RESOURCE_STATES,
        heap_props: &D3D12_HEAP_PROPERTIES,
    ) -> ID3D12Resource {
        let buf_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Alignment: 0,
            Width: size,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_UNKNOWN,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            Flags: flags,
        };
        let mut buffer: Option<ID3D12Resource> = None;
        self.device.CreateCommittedResource(heap_props, D3D12_HEAP_FLAG_NONE, &buf_desc, init_state, None, &mut buffer).unwrap();
        buffer.unwrap()
    }
    unsafe fn create_triangle_vert_buffer(&self) -> ID3D12Resource {
        let vertices = [
            vec3(0.,       1., 0.),
            vec3(0.866,  -0.5, 0.),
            vec3(-0.866, -0.5, 0.),
        ];

        // Note: using upload heaps to transfer static data like vert buffers is
        // not recommended. Every time the GPU needs it, the upload heap will be
        // marshalled over. Please read up on Default Heap usage. An upload heap
        // is used here for code simplicity and because there are very few verts
        // to actually transfer.

        // For simplicity, we create the vertex buffer on the upload heap, but that's not required
        let vertex_buffer = self.create_buffer(size_of_val(&vertices) as u64, D3D12_RESOURCE_FLAG_NONE, D3D12_RESOURCE_STATE_GENERIC_READ, &UPLOAD_HEAP_PROPS);
        let mut data = std::ptr::null_mut();
        vertex_buffer.Map(0, None, Some(&mut data)).unwrap();
        memcpy(data, vertices.as_ptr(), size_of_val(&vertices));
        vertex_buffer.Unmap(0, None);
        vertex_buffer
    }
    unsafe fn create_blas(&self, vert_buf: &ID3D12Resource) -> BLASBuffers {
        let geom_desc = D3D12_RAYTRACING_GEOMETRY_DESC {
            Type: D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
            Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
            Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
                Triangles: D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC {
                    VertexFormat: DXGI_FORMAT_R32G32B32_FLOAT,
                    VertexCount: 3,
                    VertexBuffer: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
                        StartAddress: vert_buf.GetGPUVirtualAddress(),
                        StrideInBytes: size_of::<Vec3>() as u64,
                    },
                    ..Default::default()
                }
            },
        };

        // Get the size requirements for the scratch and AS buffers
        let inputs = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
            Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL,
            Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_NONE,
            NumDescs: 1,
            DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
            Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
                pGeometryDescs: &geom_desc,
            },
        };
        let mut info = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO::default();
        self.device.GetRaytracingAccelerationStructurePrebuildInfo(&inputs, &mut info);

        // Create the buffers. They need to support UAV, and since we are going to immediately use them, we create them with an unordered-access state
        let buffers = BLASBuffers {
            scratch: self.create_buffer(info.ScratchDataSizeInBytes, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS, D3D12_RESOURCE_STATE_UNORDERED_ACCESS, &DEFAULT_HEAP_PROPS),
            result: self.create_buffer(info.ResultDataMaxSizeInBytes, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS, D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE, &DEFAULT_HEAP_PROPS),
        };

        // Create the bottom-level AS
        let as_desc = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
            DestAccelerationStructureData: buffers.result.GetGPUVirtualAddress(),
            Inputs: inputs,
            ScratchAccelerationStructureData: buffers.scratch.GetGPUVirtualAddress(),
            ..Default::default()
        };

        self.cmd_list.BuildRaytracingAccelerationStructure(&as_desc, None);

        // We need to insert a UAV barrier before using the acceleration structures in a raytracing operation
        let uav_barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                UAV: ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER{ pResource: Some(buffers.result.clone()) }),
            },
            ..Default::default()
        };
        self.cmd_list.ResourceBarrier(&[uav_barrier]);
        buffers
    }
    unsafe fn create_tlas(&mut self, blas: ID3D12Resource) -> TLASBuffers {
        // First, get the size of the TLAS buffers and create them
        let mut inputs = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
            Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
            Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_NONE,
            NumDescs: 1,
            DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
            ..Default::default()
        };
        let mut info = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO::default();
        self.device.GetRaytracingAccelerationStructurePrebuildInfo(&inputs, &mut info);

        // Create the buffers
        let mut buffers = TLASBuffers {
            scratch: self.create_buffer(info.ScratchDataSizeInBytes, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS, D3D12_RESOURCE_STATE_UNORDERED_ACCESS, &DEFAULT_HEAP_PROPS),
            result: self.create_buffer(info.ResultDataMaxSizeInBytes, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS, D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE, &DEFAULT_HEAP_PROPS),
            instance_desc: self.create_buffer(size_of::<D3D12_RAYTRACING_INSTANCE_DESC>() as u64, D3D12_RESOURCE_FLAG_NONE, D3D12_RESOURCE_STATE_GENERIC_READ, &UPLOAD_HEAP_PROPS),
        };
        self.tlas_size = info.ResultDataMaxSizeInBytes;

        // The instance desc should be inside a buffer, create and map the buffer
        let mut instance_desc = std::ptr::null_mut();
        buffers.instance_desc.Map(0, None, Some(&mut instance_desc)).unwrap();
        let instance_desc: *mut D3D12_RAYTRACING_INSTANCE_DESC = instance_desc as _;

        // Initialize the instance desc. We only have a single instance
        let m = Mat4::IDENTITY;
        memcpy((*instance_desc).Transform.as_mut_ptr(), m.as_ref(), size_of_val(&((*instance_desc).Transform)));
        (*instance_desc).AccelerationStructure = blas.GetGPUVirtualAddress();
        (*instance_desc)._bitfield1 = 0xFF000000;
        (*instance_desc)._bitfield2 = 0;

        // Unmap
        buffers.instance_desc.Unmap(0, None);

        // Create the TLAS
        inputs.Anonymous = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
             InstanceDescs: buffers.instance_desc.GetGPUVirtualAddress(),
        };
        let as_desc = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
            DestAccelerationStructureData: buffers.result.GetGPUVirtualAddress(),
            Inputs: inputs,
            ScratchAccelerationStructureData: buffers.scratch.GetGPUVirtualAddress(),
            ..Default::default()
        };

        self.cmd_list.BuildRaytracingAccelerationStructure(&as_desc, None);

        // We need to insert a UAV barrier before using the acceleration structures in a raytracing operation
        let uav_barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                UAV: ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER{ pResource: Some(buffers.result.clone()) }),
            },
            ..Default::default()
        };
        self.cmd_list.ResourceBarrier(&[uav_barrier]);
        buffers
    }
    unsafe fn create_acceleration_structures(&mut self) {
        let vert_buf = self.create_triangle_vert_buffer();
        let bottom_level_buffer = self.create_blas(&vert_buf);
        let top_level_buffer = self.create_tlas(bottom_level_buffer.result.clone());
        self.vert_buf = Some(vert_buf);

        self.submit_cmd_list();
        self.fence.SetEventOnCompletion(self.fence_value, self.fence_event).unwrap();
        WaitForSingleObject(self.fence_event, INFINITE);
        //let buffer_index = swap_chain.GetCurrentBackBufferIndex();
        self.cmd_list.Reset(&self.frame_objects[0].cmd_allocator, None).unwrap();

        self.blas = Some(bottom_level_buffer.result.clone());
        self.tlas = Some(top_level_buffer.result.clone());
    }
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
            heap: create_descriptor_heap(&device, RTV_HEAP_SIZE, D3D12_DESCRIPTOR_HEAP_TYPE_RTV, false),
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
            vert_buf: None,
            tlas: None,
            blas: None,
            tlas_size: 0,
            pipeline_state: None,
            empty_root_sig: None,
            shader_table: None,
            shader_table_entry_size: 0,
            output_resource: None,
            srv_uav_heap: None,
        }
    }
    unsafe fn on_load(hwnd: HWND, width: i32, height: i32) -> Self {
        let mut tutor = Self::init_dxr(hwnd, width, height);
        tutor.create_acceleration_structures();
        tutor.create_rt_pipeline_state();
        tutor.create_shader_resources();
        tutor.create_shader_table();
        tutor
    }
    unsafe fn begin_frame(&mut self) -> usize {
        // Bind the descriptor heaps
        self.cmd_list.SetDescriptorHeaps(&[self.srv_uav_heap.clone()]);
        self.swap_chain.GetCurrentBackBufferIndex() as usize
    }
    unsafe fn end_frame(&mut self, rtv_index: usize) {
        self.resource_barrier(self.frame_objects[rtv_index].swap_chain_buffer.clone(), D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_PRESENT);
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

        // Let's raytrace
        self.resource_barrier(self.output_resource.clone().unwrap(), D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_UNORDERED_ACCESS);
        let st_gpu_address = self.shader_table.as_ref().unwrap().GetGPUVirtualAddress();
        let raytrace_desc = D3D12_DISPATCH_RAYS_DESC {
            // RayGen is the first entry in the shader-table
            RayGenerationShaderRecord: D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
                StartAddress: st_gpu_address,
                SizeInBytes: self.shader_table_entry_size as u64,
            },
            MissShaderTable: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
                StartAddress: st_gpu_address + 1 * self.shader_table_entry_size as u64,
                SizeInBytes: self.shader_table_entry_size as u64, // Only a s single miss-entry
                StrideInBytes: self.shader_table_entry_size as u64,
            },
            HitGroupTable: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
                StartAddress: st_gpu_address + 2 * self.shader_table_entry_size as u64,
                SizeInBytes: self.shader_table_entry_size as u64,
                StrideInBytes: self.shader_table_entry_size as u64,
            },
            Width: self.swap_chain_size.x as _,
            Height: self.swap_chain_size.y as _,
            Depth: 1,
            ..Default::default()
        };

        // Bind the empty root signature
        self.cmd_list.SetComputeRootSignature(self.empty_root_sig.as_ref().unwrap());

        // Dispatch
        self.cmd_list.SetPipelineState1(self.pipeline_state.as_ref().unwrap());
        self.cmd_list.DispatchRays(&raytrace_desc);

        // Copy the results to the back-buffer
        self.resource_barrier(self.output_resource.clone().unwrap(), D3D12_RESOURCE_STATE_UNORDERED_ACCESS, D3D12_RESOURCE_STATE_COPY_SOURCE);
        self.resource_barrier(self.frame_objects[rtv_index].swap_chain_buffer.clone(), D3D12_RESOURCE_STATE_PRESENT, D3D12_RESOURCE_STATE_COPY_DEST);
        self.cmd_list.CopyResource(&self.frame_objects[rtv_index].swap_chain_buffer, self.output_resource.as_ref().unwrap());

        self.end_frame(rtv_index);
    }
    unsafe fn on_shutdown(&mut self) {
        // Wait for the command queue to finish execution
        self.fence_value += 1;
        self.cmd_queue.Signal(&self.fence, self.fence_value).unwrap();
        self.fence.SetEventOnCompletion(self.fence_value, self.fence_event).unwrap();
        WaitForSingleObject(self.fence_event, INFINITE);
    }

    unsafe fn create_shader_resources(&mut self) {
        // Create the output resource. The dimensions and format should match the swap-chain
        let res_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Alignment: 0,
            Width: self.swap_chain_size.x as _,
            Height: self.swap_chain_size.y as _,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM, // The backbuffer is actually DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, but sRGB formats can't be used with UAVs. We will convert to sRGB ourselves in the shader
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
        };
        let mut output_resource: Option<ID3D12Resource> = None;
        self.device.CreateCommittedResource(
            &DEFAULT_HEAP_PROPS,
            D3D12_HEAP_FLAG_NONE,
            &res_desc,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            None,
            &mut output_resource).unwrap();

        // Create an SRV/UAV descriptor heap. Need 2 entries - 1 SRV for the scene and 1 UAV for the output
        let srv_uav_heap = create_descriptor_heap(&self.device, 2, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, true);

        // Create the UAV. Based on the root signature we created it should be the first entry
        let uav_desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
            ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        self.device.CreateUnorderedAccessView(
            output_resource.as_ref().unwrap(),
            None,
            Some(&uav_desc),
            srv_uav_heap.GetCPUDescriptorHandleForHeapStart());

        // Create the TLAS SRV right after the UAV. Note that we are using a different SRV desc here
        let srv_desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_UNKNOWN,
            ViewDimension: D3D12_SRV_DIMENSION_RAYTRACING_ACCELERATION_STRUCTURE,
            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                RaytracingAccelerationStructure: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_SRV {
                    Location: self.tlas.as_ref().unwrap().GetGPUVirtualAddress(),
                },
            },
        };
        let mut srv_handle = srv_uav_heap.GetCPUDescriptorHandleForHeapStart();
        srv_handle.ptr += self.device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV) as usize;
        self.device.CreateShaderResourceView(None, Some(&srv_desc), srv_handle);

        self.output_resource = output_resource;
        self.srv_uav_heap = Some(srv_uav_heap);
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
    tutorial.on_shutdown();
    DestroyWindow(hwnd);
}

fn main() {
    unsafe { unsafe_main() };
}
