// Copied from Palepale crates/df/src/common/imgui_ui.rs at
// 6490070271354e2a13a30e107065338c530a7322 (build/reference/palepale).
// Adaptations: product branding/storage, capture-visible surfaces, owned teardown.
use imgui::internal::RawWrapper;
use imgui::{Context, DrawCmd, DrawData, DrawIdx, DrawVert, OwnedDrawData, TextureId};
use std::mem::size_of;
use std::path::PathBuf;
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_INDEX_BUFFER, D3D11_BIND_SHADER_RESOURCE,
    D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
    D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL,
    D3D11_CPU_ACCESS_WRITE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CULL_NONE,
    D3D11_DEPTH_STENCIL_DESC, D3D11_FILL_SOLID, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
    D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_VERTEX_DATA, D3D11_MAP_WRITE_DISCARD,
    D3D11_MAPPED_SUBRESOURCE, D3D11_RASTERIZER_DESC, D3D11_SAMPLER_DESC, D3D11_SDK_VERSION,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT, D3D11CreateDeviceAndSwapChain, ID3D11BlendState,
    ID3D11Buffer, ID3D11DepthStencilState, ID3D11Device, ID3D11DeviceContext, ID3D11InputLayout,
    ID3D11PixelShader, ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11Resource,
    ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R16_UINT,
    DXGI_FORMAT_R32_UINT, DXGI_FORMAT_R32G32_FLOAT, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGISwapChain,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWL_EXSTYLE, GetCursorPos,
    GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST, IsWindowVisible, LWA_ALPHA, MSG, PM_REMOVE,
    PeekMessageW, RegisterClassExW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, WM_CLOSE, WM_DESTROY, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{Interface, Result as WinResult, s, w};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
};

const VS: &str = r#"
cbuffer vertexBuffer : register(b0) {
    float4x4 ProjectionMatrix;
};
struct VS_INPUT {
    float2 pos : POSITION;
    float4 col : COLOR0;
    float2 uv  : TEXCOORD0;
};
struct PS_INPUT {
    float4 pos : SV_POSITION;
    float4 col : COLOR0;
    float2 uv  : TEXCOORD0;
};
PS_INPUT main(VS_INPUT input) {
    PS_INPUT output;
    output.pos = mul(ProjectionMatrix, float4(input.pos.xy, 0.f, 1.f));
    output.col = input.col;
    output.uv  = input.uv;
    return output;
}
"#;

const PS: &str = r#"
struct PS_INPUT {
    float4 pos : SV_POSITION;
    float4 col : COLOR0;
    float2 uv  : TEXCOORD0;
};
sampler sampler0;
Texture2D texture0;
float4 main(PS_INPUT input) : SV_Target {
    return input.col * texture0.Sample(sampler0, input.uv);
}
"#;

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_CLOSE || msg == WM_DESTROY {
        super::keys::request_quit();
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

pub struct Gui {
    hwnd: HWND,
    ctx: Context,
    device: ID3D11Device,
    ic: ID3D11DeviceContext,
    swap: IDXGISwapChain,
    rtv: Option<ID3D11RenderTargetView>,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    cb: ID3D11Buffer,
    blend: ID3D11BlendState,
    rast: ID3D11RasterizerState,
    depth: ID3D11DepthStencilState,
    samp: ID3D11SamplerState,
    font_srv: ID3D11ShaderResourceView,
    logo_srv: Option<ID3D11ShaderResourceView>,
    vb: Option<ID3D11Buffer>,
    ib: Option<ID3D11Buffer>,
    vb_cap: usize,
    ib_cap: usize,
    w: i32,
    h: i32,
    last: Instant,
    paint_error_logged: bool,
}

fn ini_path() -> Option<PathBuf> {
    Some(crate::common::paths::app_dir()?.join("imgui.ini"))
}

fn take<T>(hr: WinResult<()>, v: Option<T>) -> WinResult<T> {
    hr?;
    v.ok_or_else(windows::core::Error::from_win32)
}

fn present(swap: &IDXGISwapChain) -> WinResult<()> {
    unsafe { swap.Present(0, DXGI_PRESENT(8)).ok() }
}

impl Gui {
    pub fn new() -> WinResult<Self> {
        Self::create(false)
    }

    /// Ordinary capture-visible menu surface.
    pub fn new_visible() -> WinResult<Self> {
        Self::create(false)
    }

    #[allow(clippy::field_reassign_with_default)] // Retain the copied DXGI initialization.
    fn create(_capture_excluded: bool) -> WinResult<Self> {
        unsafe {
            let inst = GetModuleHandleW(None)?;
            let class = w!("ExternalEverythingImgui");
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: inst.into(),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);
            let (sw, sh) = super::overlay::display_size();
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE
                    | WS_EX_TRANSPARENT,
                class,
                w!("External Everything Menu"),
                WS_POPUP,
                0,
                0,
                sw,
                sh,
                None,
                None,
                inst,
                None,
            )?;
            SetLayeredWindowAttributes(
                hwnd,
                windows::Win32::Foundation::COLORREF(0),
                255,
                LWA_ALPHA,
            )?;
            let margins = MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);

            let mut sd = DXGI_SWAP_CHAIN_DESC::default();
            sd.BufferCount = 2;
            sd.BufferDesc.Width = sw.max(64) as u32;
            sd.BufferDesc.Height = sh.max(64) as u32;
            sd.BufferDesc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
            sd.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
            sd.OutputWindow = hwnd;
            sd.SampleDesc.Count = 1;
            sd.Windowed = true.into();
            sd.SwapEffect = DXGI_SWAP_EFFECT_DISCARD;

            let levels = [
                D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_10_1,
                D3D_FEATURE_LEVEL_10_0,
            ];
            let mut device = None;
            let mut ic = None;
            let mut swap = None;
            let mut level = D3D_FEATURE_LEVEL::default();
            D3D11CreateDeviceAndSwapChain(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&sd),
                Some(&mut swap),
                Some(&mut device),
                Some(&mut level),
                Some(&mut ic),
            )?;
            let device = device.ok_or_else(windows::core::Error::from_win32)?;
            let ic = ic.ok_or_else(windows::core::Error::from_win32)?;
            let swap = swap.ok_or_else(windows::core::Error::from_win32)?;
            let _ = level;

            let vs_blob = compile(VS, s!("vs_4_0"))?;
            let ps_blob = compile(PS, s!("ps_4_0"))?;
            let vs_bin = blob_bytes(&vs_blob);
            let ps_bin = blob_bytes(&ps_blob);
            let mut vs = None;
            let vs = take(device.CreateVertexShader(vs_bin, None, Some(&mut vs)), vs)?;
            let mut ps = None;
            let ps = take(device.CreatePixelShader(ps_bin, None, Some(&mut ps)), ps)?;
            let layout_desc = [
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: s!("POSITION"),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: 0,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: s!("TEXCOORD"),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: 8,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
                D3D11_INPUT_ELEMENT_DESC {
                    SemanticName: s!("COLOR"),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    InputSlot: 0,
                    AlignedByteOffset: 16,
                    InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
            ];
            let mut layout = None;
            let layout = take(
                device.CreateInputLayout(&layout_desc, vs_bin, Some(&mut layout)),
                layout,
            )?;

            let cb = create_buf(&device, 64, D3D11_BIND_CONSTANT_BUFFER.0 as u32, None)?;
            let mut bd = D3D11_BLEND_DESC::default();
            bd.RenderTarget[0].BlendEnable = true.into();
            bd.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
            bd.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
            bd.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
            bd.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
            bd.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
            bd.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
            bd.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
            let mut blend = None;
            let blend = take(device.CreateBlendState(&bd, Some(&mut blend)), blend)?;

            let rast_desc = D3D11_RASTERIZER_DESC {
                FillMode: D3D11_FILL_SOLID,
                CullMode: D3D11_CULL_NONE,
                ScissorEnable: true.into(),
                DepthClipEnable: true.into(),
                ..Default::default()
            };
            let mut rast = None;
            let rast = take(
                device.CreateRasterizerState(&rast_desc, Some(&mut rast)),
                rast,
            )?;

            let depth_desc = D3D11_DEPTH_STENCIL_DESC {
                DepthEnable: false.into(),
                ..Default::default()
            };
            let mut depth = None;
            let depth = take(
                device.CreateDepthStencilState(&depth_desc, Some(&mut depth)),
                depth,
            )?;

            let samp_desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            let mut samp = None;
            let samp = take(device.CreateSamplerState(&samp_desc, Some(&mut samp)), samp)?;

            let mut ctx = Context::create();
            ctx.set_ini_filename(ini_path());
            crate::common::theme::apply(ctx.style_mut());
            let font_srv = upload_fonts(&device, &mut ctx)?;
            let logo_srv = upload_logo(&device).ok();

            Ok(Self {
                hwnd,
                ctx,
                device,
                ic,
                swap,
                rtv: None,
                vs,
                ps,
                layout,
                cb,
                blend,
                rast,
                depth,
                samp,
                font_srv,
                logo_srv,
                vb: None,
                ib: None,
                vb_cap: 0,
                ib_cap: 0,
                w: sw,
                h: sh,
                last: Instant::now(),
                paint_error_logged: false,
            })
        }
    }

    pub fn pump(&self) {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, self.hwnd, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    pub fn sync(&mut self, overlay: &super::overlay::Overlay) {
        let (x, y, w, h) = overlay.screen_rect();
        let w = w.max(64);
        let h = h.max(64);
        let size_changed = w != self.w || h != self.h;
        let mut rect = RECT::default();
        let placed = unsafe { GetWindowRect(self.hwnd, &mut rect) }.is_ok();
        if !placed
            || rect.left != x
            || rect.top != y
            || rect.right - rect.left != w
            || rect.bottom - rect.top != h
        {
            unsafe {
                let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);
            }
        }
        if size_changed {
            unsafe {
                self.ic.OMSetRenderTargets(None, None);
            }
            self.rtv = None;
            let result = unsafe {
                self.swap.ResizeBuffers(
                    0,
                    w as u32,
                    h as u32,
                    DXGI_FORMAT::default(),
                    DXGI_SWAP_CHAIN_FLAG(0),
                )
            };
            match result {
                Ok(()) => {
                    self.w = w;
                    self.h = h;
                }
                Err(error) => {
                    super::olog::line(&format!("menu ResizeBuffers failed: {error}"));
                }
            }
        }
    }

    pub fn pulse_top(&self) {
        if !unsafe { IsWindowVisible(self.hwnd) }.as_bool() {
            return;
        }
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    pub fn blank_and_hide(&mut self) {
        if self.ensure_rtv().is_ok() {
            unsafe {
                if let Some(rtv) = &self.rtv {
                    self.ic.ClearRenderTargetView(rtv, &[0.0, 0.0, 0.0, 0.0]);
                }
            }
            if let Err(error) = present(&self.swap) {
                super::olog::line(&format!("menu clear Present failed: {error}"));
            }
        }
        self.hide();
    }

    pub fn set_click_through(&self, through: bool) {
        unsafe {
            let style = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let bit = WS_EX_TRANSPARENT.0 as isize;
            let next = if through { style | bit } else { style & !bit };
            if next != style {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, next);
            }
        }
    }

    pub fn frame(
        &mut self,
        _overlay: &super::overlay::Overlay,
        f: impl FnOnce(&imgui::Ui),
    ) -> bool {
        let dt = self.last.elapsed().as_secs_f32().clamp(1.0 / 1000.0, 0.1);
        self.last = Instant::now();
        let (mx, my) = self.cursor();
        {
            let io = self.ctx.io_mut();
            io.display_size = [self.w as f32, self.h as f32];
            io.delta_time = dt;
            io.mouse_pos = [mx, my];
            io.mouse_down[0] = key(VK_LBUTTON);
            io.mouse_down[1] = key(VK_RBUTTON);
            io.mouse_down[2] = key(VK_MBUTTON);
        }
        crate::common::theme::apply(self.ctx.style_mut());
        let want = {
            let ui = self.ctx.new_frame();
            f(ui);
            ui.io().want_capture_mouse
        };
        let owned = OwnedDrawData::from(self.ctx.render());
        if let Some(data) = owned.draw_data() {
            match self.paint(data) {
                Ok(()) => self.paint_error_logged = false,
                Err(error) if !self.paint_error_logged => {
                    super::olog::line(&format!("menu paint failed: {error}"));
                    self.paint_error_logged = true;
                }
                Err(_) => {}
            }
        }
        want
    }

    pub fn paint_failed(&self) -> bool {
        self.paint_error_logged
    }

    pub fn surface_ready(&self) -> bool {
        self.rtv.is_some()
    }

    fn cursor(&self) -> (f32, f32) {
        unsafe {
            let mut pt = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut pt).is_err() {
                return (-1.0, -1.0);
            }
            let _ = ScreenToClient(self.hwnd, &mut pt);
            (pt.x as f32, pt.y as f32)
        }
    }

    fn ensure_rtv(&mut self) -> WinResult<()> {
        if self.rtv.is_some() {
            return Ok(());
        }
        unsafe {
            let bb: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            let mut rtv = None;
            self.rtv = Some(take(
                self.device
                    .CreateRenderTargetView(&bb, None, Some(&mut rtv)),
                rtv,
            )?);
        }
        Ok(())
    }

    fn grow_bufs(&mut self, vtx: usize, idx: usize) -> WinResult<()> {
        if vtx > self.vb_cap {
            let cap = vtx + 5000;
            self.vb = Some(create_buf(
                &self.device,
                (cap * size_of::<DrawVert>()) as u32,
                D3D11_BIND_VERTEX_BUFFER.0 as u32,
                None,
            )?);
            self.vb_cap = cap;
        }
        if idx > self.ib_cap {
            let cap = idx + 10000;
            self.ib = Some(create_buf(
                &self.device,
                (cap * size_of::<DrawIdx>()) as u32,
                D3D11_BIND_INDEX_BUFFER.0 as u32,
                None,
            )?);
            self.ib_cap = cap;
        }
        Ok(())
    }

    fn paint(&mut self, draw: &DrawData) -> WinResult<()> {
        self.ensure_rtv()?;
        if draw.total_vtx_count <= 0 || draw.display_size[0] <= 0.0 || draw.display_size[1] <= 0.0 {
            unsafe {
                if let Some(rtv) = &self.rtv {
                    self.ic.ClearRenderTargetView(rtv, &[0.0, 0.0, 0.0, 0.0]);
                }
            }
            return present(&self.swap);
        }
        self.grow_bufs(draw.total_vtx_count as usize, draw.total_idx_count as usize)?;
        let Some(vb) = self.vb.clone() else {
            return Ok(());
        };
        let Some(ib) = self.ib.clone() else {
            return Ok(());
        };
        unsafe {
            map_copy(&self.ic, &vb, |dst| {
                let mut off = 0usize;
                for list in draw.draw_lists() {
                    let src = list.vtx_buffer();
                    let n = std::mem::size_of_val(src);
                    std::ptr::copy_nonoverlapping(src.as_ptr() as *const u8, dst.add(off), n);
                    off += n;
                }
            })?;
            map_copy(&self.ic, &ib, |dst| {
                let mut off = 0usize;
                for list in draw.draw_lists() {
                    let src = list.idx_buffer();
                    let n = std::mem::size_of_val(src);
                    std::ptr::copy_nonoverlapping(src.as_ptr() as *const u8, dst.add(off), n);
                    off += n;
                }
            })?;

            let l = draw.display_pos[0];
            let r = draw.display_pos[0] + draw.display_size[0];
            let t = draw.display_pos[1];
            let b = draw.display_pos[1] + draw.display_size[1];
            let mvp = [
                [2.0 / (r - l), 0.0, 0.0, 0.0],
                [0.0, 2.0 / (t - b), 0.0, 0.0],
                [0.0, 0.0, 0.5, 0.0],
                [(r + l) / (l - r), (t + b) / (b - t), 0.5, 1.0],
            ];
            map_copy(&self.ic, &self.cb, |dst| {
                std::ptr::copy_nonoverlapping(mvp.as_ptr() as *const u8, dst, 64);
            })?;

            if let Some(rtv) = &self.rtv {
                self.ic.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
                self.ic.ClearRenderTargetView(rtv, &[0.0, 0.0, 0.0, 0.0]);
            }
            let vp = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: draw.display_size[0],
                Height: draw.display_size[1],
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            self.ic.RSSetViewports(Some(&[vp]));
            self.ic.IASetInputLayout(&self.layout);
            let stride = size_of::<DrawVert>() as u32;
            let offset = 0u32;
            let slot = Some(vb);
            self.ic
                .IASetVertexBuffers(0, 1, Some(&slot as *const _), Some(&stride), Some(&offset));
            let idx_fmt = if size_of::<DrawIdx>() == 2 {
                DXGI_FORMAT_R16_UINT
            } else {
                DXGI_FORMAT_R32_UINT
            };
            self.ic.IASetIndexBuffer(&ib, idx_fmt, 0);
            self.ic
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.ic.VSSetShader(&self.vs, None);
            self.ic.PSSetShader(&self.ps, None);
            self.ic
                .VSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            self.ic.PSSetSamplers(0, Some(&[Some(self.samp.clone())]));
            self.ic
                .OMSetBlendState(&self.blend, Some(&[0.0, 0.0, 0.0, 0.0]), 0xffffffff);
            self.ic.OMSetDepthStencilState(&self.depth, 0);
            self.ic.RSSetState(&self.rast);

            let clip_off = draw.display_pos;
            let fb_w = draw.display_size[0];
            let fb_h = draw.display_size[1];
            let mut global_vtx = 0i32;
            let mut global_idx = 0u32;
            for list in draw.draw_lists() {
                for cmd in list.commands() {
                    match cmd {
                        DrawCmd::Elements { count, cmd_params } => {
                            let cr = cmd_params.clip_rect;
                            let x1 = (cr[0] - clip_off[0]) as i32;
                            let y1 = (cr[1] - clip_off[1]) as i32;
                            let x2 = (cr[2] - clip_off[0]) as i32;
                            let y2 = (cr[3] - clip_off[1]) as i32;
                            if x2 <= 0 || y2 <= 0 || x1 >= fb_w as i32 || y1 >= fb_h as i32 {
                                continue;
                            }
                            let sc = RECT {
                                left: x1.max(0),
                                top: y1.max(0),
                                right: x2.min(fb_w as i32),
                                bottom: y2.min(fb_h as i32),
                            };
                            self.ic.RSSetScissorRects(Some(&[sc]));
                            let srv = if cmd_params.texture_id == TextureId::new(2) {
                                self.logo_srv.as_ref().unwrap_or(&self.font_srv)
                            } else {
                                &self.font_srv
                            };
                            self.ic.PSSetShaderResources(0, Some(&[Some(srv.clone())]));
                            self.ic.DrawIndexed(
                                count as u32,
                                global_idx + cmd_params.idx_offset as u32,
                                global_vtx + cmd_params.vtx_offset as i32,
                            );
                        }
                        DrawCmd::ResetRenderState => {}
                        DrawCmd::RawCallback { callback, raw_cmd } => {
                            callback(list.raw(), raw_cmd);
                        }
                    }
                }
                global_vtx += list.vtx_buffer().len() as i32;
                global_idx += list.idx_buffer().len() as u32;
            }
        }
        present(&self.swap)
    }
}

impl Drop for Gui {
    fn drop(&mut self) {
        self.hide();
        unsafe {
            self.ic.OMSetRenderTargets(None, None);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn key(vk: u16) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) < 0 }
}

fn blob_bytes(blob: &windows::Win32::Graphics::Direct3D::ID3DBlob) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize())
    }
}

fn compile(
    src: &str,
    target: windows::core::PCSTR,
) -> WinResult<windows::Win32::Graphics::Direct3D::ID3DBlob> {
    unsafe {
        let mut blob = None;
        let hr = D3DCompile(
            src.as_ptr() as *const _,
            src.len(),
            None,
            None,
            None,
            s!("main"),
            target,
            0,
            0,
            &mut blob,
            None,
        );
        if hr.is_err() {
            hr?;
        }
        blob.ok_or_else(windows::core::Error::from_win32)
    }
}

fn create_buf(
    device: &ID3D11Device,
    bytes: u32,
    bind: u32,
    init: Option<&D3D11_SUBRESOURCE_DATA>,
) -> WinResult<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: bytes,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: bind,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        ..Default::default()
    };
    let mut buf = None;
    take(
        unsafe { device.CreateBuffer(&desc, init.map(|p| p as *const _), Some(&mut buf)) },
        buf,
    )
}

fn map_copy<F: FnOnce(*mut u8)>(
    ic: &ID3D11DeviceContext,
    buf: &ID3D11Buffer,
    f: F,
) -> WinResult<()> {
    unsafe {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        ic.Map(
            &buf.cast::<ID3D11Resource>()?,
            0,
            D3D11_MAP_WRITE_DISCARD,
            0,
            Some(&mut mapped),
        )?;
        f(mapped.pData as *mut u8);
        ic.Unmap(&buf.cast::<ID3D11Resource>()?, 0);
    }
    Ok(())
}

fn upload_fonts(device: &ID3D11Device, ctx: &mut Context) -> WinResult<ID3D11ShaderResourceView> {
    let fonts = ctx.fonts();
    let tex = fonts.build_rgba32_texture();
    let desc = D3D11_TEXTURE2D_DESC {
        Width: tex.width,
        Height: tex.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        ..Default::default()
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: tex.data.as_ptr() as *const _,
        SysMemPitch: tex.width * 4,
        SysMemSlicePitch: 0,
    };
    let mut gpu = None;
    let gpu = take(
        unsafe { device.CreateTexture2D(&desc, Some(&init), Some(&mut gpu)) },
        gpu,
    )?;
    let mut srv = None;
    let srv = take(
        unsafe { device.CreateShaderResourceView(&gpu, None, Some(&mut srv)) },
        srv,
    )?;
    fonts.tex_id = TextureId::new(1);
    Ok(srv)
}

const LOGO_W: u32 = 64;
const LOGO_H: u32 = 64;
const LOGO_RGBA: &[u8] = include_bytes!("../../assets/palepale_logo.rgba");

fn upload_logo(device: &ID3D11Device) -> WinResult<ID3D11ShaderResourceView> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: LOGO_W,
        Height: LOGO_H,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        ..Default::default()
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: LOGO_RGBA.as_ptr() as *const _,
        SysMemPitch: LOGO_W * 4,
        SysMemSlicePitch: 0,
    };
    let mut gpu = None;
    let gpu = take(
        unsafe { device.CreateTexture2D(&desc, Some(&init), Some(&mut gpu)) },
        gpu,
    )?;
    let mut srv = None;
    take(
        unsafe { device.CreateShaderResourceView(&gpu, None, Some(&mut srv)) },
        srv,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an interactive Windows graphics session"]
    fn window_menu_first_frame_and_teardown() -> WinResult<()> {
        let overlay = super::super::overlay::Overlay::new_visible()?;
        let mut gui = Gui::new_visible()?;
        gui.sync(&overlay);
        let _ = gui.frame(&overlay, |_| {});
        assert!(gui.surface_ready());
        drop(gui);
        drop(overlay);
        Ok(())
    }
}
