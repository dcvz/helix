use log::trace;
use wgpu::CompareFunction;

use super::{
    gbi::defines::Viewport,
    gfx_device::GfxDevice,
    rcp::RCP,
    rsp::{RSPGeometry, StagingVertex, RSP},
    utils::{
        color_combiner::{
            ColorCombiner, ColorCombinerManager, CombineParams, ACMUX, CCMUX, SHADER,
        },
        texture::{Texture, TextureManager},
    },
};
use crate::graphics::gfx_device::ShaderProgram;

pub const SCREEN_WIDTH: f32 = 320.0;
pub const SCREEN_HEIGHT: f32 = 240.0;
const MAX_VBO_SIZE: usize = 256;
const TEXTURE_CACHE_MAX_SIZE: usize = 500;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub const ZERO: Self = Self {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    };

    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OutputDimensions {
    pub width: u32,
    pub height: u32,
    pub aspect_ratio: f32,
}

impl OutputDimensions {
    pub const ZERO: Self = Self {
        width: 0,
        height: 0,
        aspect_ratio: 0.0,
    };
}

pub struct RenderingState {
    pub depth_compare: CompareFunction,
    pub depth_test: bool,
    pub depth_write: bool,

    pub decal_mode: bool,
    pub alpha_blend: bool,
    pub viewport: Rect,
    pub scissor: Rect,
    pub shader_program: *mut ShaderProgram,
    pub textures: [Texture; 2],
}

impl RenderingState {
    pub const EMPTY: Self = Self {
        depth_compare: CompareFunction::Always,
        depth_test: false,
        depth_write: false,
        decal_mode: false,
        alpha_blend: false,
        viewport: Rect::ZERO,
        scissor: Rect::ZERO,
        shader_program: std::ptr::null_mut(),
        textures: [Texture::EMPTY; 2],
    };
}

enum OtherModeLayoutL {
    // non-render-mode fields
    G_MDSFT_ALPHACOMPARE = 0,
    G_MDSFT_ZSRCSEL = 2,
    // cycle-independent render-mode bits
    AA_EN = 3,
    Z_CMP = 4,
    Z_UPD = 5,
    IM_RD = 6,
    CLR_ON_CVG = 7,
    CVG_DST = 8,
    ZMODE = 10,
    CVG_X_ALPHA = 12,
    ALPHA_CVG_SEL = 13,
    FORCE_BL = 14,
    // bit 15 unused, was "TEX_EDGE"
    // cycle-dependent render-mode bits
    B_2 = 16,
    B_1 = 18,
    M_2 = 20,
    M_1 = 22,
    A_2 = 24,
    A_1 = 26,
    P_2 = 28,
    P_1 = 30,
}
enum ZMode {
    ZMODE_OPA = 0,
    ZMODE_INTER = 1,
    ZMODE_XLU = 2, // translucent
    ZMODE_DEC = 3,
}

enum BlendParamPMColor {
    G_BL_CLR_IN = 0,
    G_BL_CLR_MEM = 1,
    G_BL_CLR_BL = 2,
    G_BL_CLR_FOG = 3,
}

enum BlendParamA {
    G_BL_A_IN = 0,
    G_BL_A_FOG = 1,
    G_BL_A_SHADE = 2,
    G_BL_0 = 3,
}

enum BlendParamB {
    G_BL_1MA = 0,
    G_BL_A_MEM = 1,
    G_BL_1 = 2,
    G_BL_0 = 3,
}

pub struct RDP {
    pub output_dimensions: OutputDimensions,
    pub rendering_state: RenderingState,

    pub texture_manager: TextureManager,
    pub color_combiner_manager: ColorCombinerManager,

    pub viewport: Rect,
    pub scissor: Rect,
    pub viewport_or_scissor_changed: bool,

    pub combine: CombineParams,
    pub other_mode_l: u32,
    pub other_mode_h: u32,

    pub buf_vbo: [f32; MAX_VBO_SIZE * (26 * 3)], // 3 vertices in a triangle and 26 floats per vtx
    pub buf_vbo_len: usize,
    pub buf_vbo_num_tris: usize,
}

impl RDP {
    pub fn new() -> Self {
        RDP {
            output_dimensions: OutputDimensions::ZERO,
            rendering_state: RenderingState::EMPTY,

            texture_manager: TextureManager::new(TEXTURE_CACHE_MAX_SIZE),
            color_combiner_manager: ColorCombinerManager::new(),

            viewport: Rect::ZERO,
            scissor: Rect::ZERO,
            viewport_or_scissor_changed: false,

            combine: CombineParams::ZERO,
            other_mode_l: 0,
            other_mode_h: 0,

            buf_vbo: [0.0; MAX_VBO_SIZE * (26 * 3)],
            buf_vbo_len: 0,
            buf_vbo_num_tris: 0,
        }
    }

    pub fn reset(&mut self) {}

    pub fn calculate_and_set_viewport(&mut self, viewport: Viewport) {
        let mut width = 2.0 * viewport.vscale[0] as f32 / 4.0;
        let mut height = 2.0 * viewport.vscale[1] as f32 / 4.0;
        let mut x = viewport.vtrans[0] as f32 / 4.0 - width / 2.0;
        let mut y = viewport.vtrans[1] as f32 / 4.0 - height / 2.0;

        width *= self.scaled_x();
        height *= self.scaled_y();
        x *= self.scaled_x();
        y *= self.scaled_y();

        self.viewport.x = x as u16;
        self.viewport.y = y as u16;
        self.viewport.width = width as u16;
        self.viewport.height = height as u16;

        self.viewport_or_scissor_changed = true;
    }

    pub fn adjust_x_for_viewport(&self, x: f32) -> f32 {
        x * (4.0 / 3.0)
            / (self.output_dimensions.width as f32 / self.output_dimensions.height as f32)
    }

    pub fn flush(&mut self, gfx_device: &GfxDevice) {
        if self.buf_vbo_len > 0 {
            gfx_device.draw_triangles(
                &self.buf_vbo as *const f32,
                self.buf_vbo_len,
                self.buf_vbo_num_tris,
            );
            self.buf_vbo_len = 0;
            self.buf_vbo_num_tris = 0;
        }
    }

    pub fn lookup_or_create_shader_program(
        &mut self,
        gfx_device: &GfxDevice,
        shader_id: u32,
    ) -> *mut ShaderProgram {
        let mut shader_program = gfx_device.lookup_shader(shader_id);
        if shader_program.is_null() {
            gfx_device.unload_shader(self.rendering_state.shader_program);
            shader_program = gfx_device.create_and_load_new_shader(shader_id);
            self.rendering_state.shader_program = shader_program;
        }

        shader_program
    }

    pub fn create_color_combiner(&mut self, gfx_device: &GfxDevice, cc_id: u32) -> &ColorCombiner {
        self.flush(gfx_device);
        self.generate_color_combiner(gfx_device, cc_id);

        let combiner = self.color_combiner_manager.combiners.get(&cc_id).unwrap();
        self.color_combiner_manager.current_combiner = Some(cc_id);

        combiner
    }

    pub fn lookup_or_create_color_combiner(&mut self, gfx_device: &GfxDevice, cc_id: u32) {
        if let Some(_cc) = self.color_combiner_manager.lookup_color_combiner(cc_id) {
        } else {
            self.create_color_combiner(gfx_device, cc_id);
        }
    }

    pub fn generate_color_combiner(&mut self, gfx_device: &GfxDevice, cc_id: u32) {
        let mut shader_id = (cc_id >> 24) << 24;
        let mut shader_input_mapping = [[0u8; 4]; 2];

        // parse the color combine pass
        {
            let mut input_number = [0u8; 8];
            let mut next_input_number = SHADER::INPUT_1 as u8;

            for i in 0..4 {
                let mut val = 0;
                match self.combine.c0.get(i) {
                    CCMUX::TEXEL0 => val = SHADER::TEXEL0 as u8,
                    CCMUX::TEXEL1 => val = SHADER::TEXEL1 as u8,
                    CCMUX::TEXEL0_ALPHA => val = SHADER::TEXEL0A as u8,
                    CCMUX::PRIMITIVE | CCMUX::SHADE | CCMUX::ENVIRONMENT | CCMUX::LOD_FRACTION => {
                        let property = self.combine.c0.get(i) as u8;

                        if input_number[property as usize] == 0 {
                            shader_input_mapping[0][(next_input_number - 1) as usize] = property;
                            input_number[property as usize] = next_input_number;
                            next_input_number += 1;
                        }
                        val = input_number[property as usize];
                    }
                    _ => {}
                }

                shader_id |= (val as u32) << (i * 3);
            }
        }

        // parse the alpha combine pass
        {
            let mut input_number = [0u8; 8];
            let mut next_input_number = SHADER::INPUT_1 as u8;

            for i in 0..4 {
                let mut val = 0;
                match self.combine.a0.get(i) {
                    ACMUX::TEXEL0 => val = SHADER::TEXEL0 as u8,
                    ACMUX::TEXEL1 => val = SHADER::TEXEL1 as u8,
                    ACMUX::PRIMITIVE | ACMUX::SHADE | ACMUX::ENVIRONMENT => {
                        let property = self.combine.a0.get(i) as u8;

                        if input_number[property as usize] == 0 {
                            shader_input_mapping[1][(next_input_number - 1) as usize] = property;
                            input_number[property as usize] = next_input_number;
                            next_input_number += 1;
                        }
                        val = input_number[property as usize];
                    }
                    _ => {}
                }

                shader_id |= (val as u32) << (12 + i * 3);
            }
        }

        let shader_program = self.lookup_or_create_shader_program(gfx_device, shader_id);
        let combiner = ColorCombiner::new(shader_id, shader_program, shader_input_mapping);
        self.color_combiner_manager
            .combiners
            .insert(cc_id, combiner);
    }

    fn translate_blend_mode(&mut self, gfx_device: &GfxDevice, render_mode: u32) {
        let src_color = render_mode >> OtherModeLayoutL::P_2 as u32 & 0x03;
        let src_factor = render_mode >> OtherModeLayoutL::A_2 as u32 & 0x03;
        let dst_color = render_mode >> OtherModeLayoutL::M_2 as u32 & 0x03;
        let dst_factor = render_mode >> OtherModeLayoutL::B_2 as u32 & 0x03;

        // handle depth compare
        if self.other_mode_l & (1 << OtherModeLayoutL::Z_CMP as u32) != 0 {
            let zmode = self.other_mode_l >> (OtherModeLayoutL::ZMODE as u32) & 0x03;
            let depth_compare = match zmode {
                x if x == ZMode::ZMODE_OPA as u32 => CompareFunction::Less,
                x if x == ZMode::ZMODE_INTER as u32 => CompareFunction::Less, // TODO: Understand this
                x if x == ZMode::ZMODE_XLU as u32 => CompareFunction::Less,
                x if x == ZMode::ZMODE_DEC as u32 => CompareFunction::LessEqual,
                _ => panic!("Unknown ZMode"),
            };

            if depth_compare != self.rendering_state.depth_compare {
                self.flush(&gfx_device);
                gfx_device.set_depth_compare(depth_compare as u8);
                self.rendering_state.depth_compare = depth_compare;
            }
        }

        // handle depth write
        let depth_write = render_mode & (1 << OtherModeLayoutL::Z_UPD as u32) != 0;
        if depth_write != self.rendering_state.depth_write {
            self.flush(&gfx_device);
            gfx_device.set_depth_write(depth_write);
            self.rendering_state.depth_write = depth_write;
        }
    }

    pub fn update_render_state(
        &mut self,
        gfx_device: &GfxDevice,
        geometry_mode: u32,
        vertices: &[&StagingVertex; 3],
    ) {
        let depth_test = geometry_mode & RSPGeometry::G_ZBUFFER as u32 != 0;
        if depth_test != self.rendering_state.depth_test {
            self.flush(gfx_device);
            gfx_device.set_depth_test(depth_test);
            self.rendering_state.depth_test = depth_test;
        }

        self.translate_blend_mode(gfx_device, self.other_mode_l);

        // TODO: Rename depth_test to depth_write
        // let z_upd = self.other_mode_l & (1 << OtherModeLayoutL::Z_UPD as u32) != 0;
        // if z_upd != self.rendering_state.depth_mask {
        //     self.flush(gfx_device);
        //     gfx_device.set_depth_mask(z_upd);
        //     self.rendering_state.depth_mask = z_upd;
        // }

        // let zmode = self.other_mode_l >> (OtherModeLayoutL::ZMODE as u32) & 0x03;
        // // TODO: Rename zmode_decal to polygon_offset
        // let zmode_decal = zmode == ZMode::ZMODE_DEC as u32;
        // if zmode_decal != self.rendering_state.decal_mode {
        //     self.flush(gfx_device);
        //     gfx_device.set_zmode_decal(zmode_decal);
        //     self.rendering_state.decal_mode = zmode_decal;
        // }

        // if self.viewport_or_scissor_changed {
        //     let viewport = self.viewport;
        //     if viewport != self.rendering_state.viewport {
        //         self.flush(gfx_device);
        //         gfx_device.set_viewport(viewport.x as i32, viewport.y as i32, viewport.width as i32, viewport.height as i32);
        //         self.rendering_state.viewport = viewport;
        //     }
        //     let scissor = self.scissor;
        //     if scissor != self.rendering_state.scissor {
        //         self.flush(gfx_device);
        //         gfx_device.set_scissor(scissor.x as i32, scissor.y as i32, scissor.width as i32, scissor.height as i32);
        //         self.rendering_state.scissor = scissor;
        //     }
        //     self.viewport_or_scissor_changed = false;
        // }

        // let cc_id = self.combine_mode;

        // // bool use_alpha = (RDPGetOtherModeL(rcp) & (G_BL_A_MEM << 18)) == 0;
        // // bool use_fog = (RDPGetOtherModeL(rcp) >> 30) == G_BL_CLR_FOG;
        // // bool texture_edge = (RDPGetOtherModeL(rcp) & CVG_X_ALPHA) == CVG_X_ALPHA;
        // // bool use_noise = (RDPGetOtherModeL(rcp) & G_AC_DITHER) == G_AC_DITHER;

        // let use_alpha = (self.other_mode_l >> OtherModeLayoutL::B_1 as u32) & 0x03 != BlendParamB::G_BL_A_MEM as u32;
        // let use_fog = (self.other_mode_l >> OtherModeLayoutL::P_1 as u32) & 0x03 == BlendParamPMColor::G_BL_CLR_FOG as u32;
        // let texture_edge = (self.other_mode_l & (1 << OtherModeLayoutL::CVG_X_ALPHA as u32)) != 0;
    }

    // MARK: - Helpers

    pub fn scaled_x(&self) -> f32 {
        self.output_dimensions.width as f32 / SCREEN_WIDTH
    }

    pub fn scaled_y(&self) -> f32 {
        self.output_dimensions.height as f32 / SCREEN_HEIGHT
    }
}

// MARK: - C Bridge

#[no_mangle]
pub extern "C" fn RDPFlush(rcp: Option<&mut RCP>) {
    let rcp = rcp.unwrap();
    rcp.rdp.flush(rcp.gfx_device.as_ref().unwrap());
}

#[no_mangle]
pub extern "C" fn RDPLookupOrCreateColorCombiner(rcp: Option<&mut RCP>, cc_id: u32) {
    let rcp = rcp.unwrap();
    rcp.rdp
        .lookup_or_create_color_combiner(rcp.gfx_device.as_ref().unwrap(), cc_id);
}

#[no_mangle]
pub extern "C" fn RDPAddToVBOAndIncrement(rcp: Option<&mut RCP>, value: f32) {
    let rcp = rcp.unwrap();
    rcp.rdp.buf_vbo[rcp.rdp.buf_vbo_len] = value;
    rcp.rdp.buf_vbo_len += 1;
}

#[no_mangle]
pub extern "C" fn RDPIncrementTriangleCountAndReturn(rcp: Option<&mut RCP>) -> usize {
    let rcp = rcp.unwrap();
    rcp.rdp.buf_vbo_num_tris += 1;
    rcp.rdp.buf_vbo_num_tris
}

#[no_mangle]
pub extern "C" fn RDPSetRenderingStateViewport(rcp: Option<&mut RCP>, viewport: Rect) {
    let rcp = rcp.unwrap();
    rcp.rdp.rendering_state.viewport = viewport;
}

#[no_mangle]
pub extern "C" fn RDPSetRenderingStateScissor(rcp: Option<&mut RCP>, scissor: Rect) {
    let rcp = rcp.unwrap();
    rcp.rdp.rendering_state.scissor = scissor;
}

#[no_mangle]
pub extern "C" fn RDPLookupOrCreateShaderProgram(rcp: Option<&mut RCP>, shader_id: u32) {
    let rcp = rcp.unwrap();
    rcp.rdp
        .lookup_or_create_shader_program(rcp.gfx_device.as_ref().unwrap(), shader_id);
}

#[no_mangle]
pub extern "C" fn RDPSetRenderingStateDepthMask(rcp: Option<&mut RCP>, value: bool) {
    let rcp = rcp.unwrap();
    rcp.rdp.rendering_state.depth_write = value;
}

#[no_mangle]
pub extern "C" fn RDPGetRenderingStateZModeDecal(rcp: Option<&mut RCP>) -> bool {
    let rcp = rcp.unwrap();
    rcp.rdp.rendering_state.decal_mode
}

#[no_mangle]
pub extern "C" fn RDPSetRenderingStateZModeDecal(rcp: Option<&mut RCP>, value: bool) {
    let rcp = rcp.unwrap();
    rcp.rdp.rendering_state.decal_mode = value;
}

#[no_mangle]
pub extern "C" fn RDPGetRenderingStateUseAlpha(rcp: Option<&mut RCP>) -> bool {
    let rcp = rcp.unwrap();
    rcp.rdp.rendering_state.alpha_blend
}

#[no_mangle]
pub extern "C" fn RDPSetRenderingStateUseAlpha(rcp: Option<&mut RCP>, value: bool) {
    let rcp = rcp.unwrap();
    rcp.rdp.rendering_state.alpha_blend = value;
}

#[no_mangle]
pub extern "C" fn RDPGetRenderingStateShaderProgram(rcp: Option<&mut RCP>) -> *const ShaderProgram {
    let rcp = rcp.unwrap();
    rcp.rdp.rendering_state.shader_program
}

#[no_mangle]
pub extern "C" fn RDPSetRenderingStateShaderProgram(
    rcp: Option<&mut RCP>,
    prg: *mut ShaderProgram,
) {
    let rcp = rcp.unwrap();
    rcp.rdp.rendering_state.shader_program = prg;
}

#[no_mangle]
pub extern "C" fn RDPGetRenderingStateTextureAtIndex(
    rcp: Option<&mut RCP>,
    index: usize,
) -> *const Texture {
    let rcp = rcp.unwrap();
    Box::into_raw(Box::new(rcp.rdp.rendering_state.textures[index]))
}

#[no_mangle]
pub extern "C" fn RDPViewportDoesNotEqualRenderingStateViewport(rcp: Option<&mut RCP>) -> bool {
    let rcp = rcp.unwrap();
    rcp.rdp.rendering_state.viewport != rcp.rdp.viewport
}

#[no_mangle]
pub extern "C" fn RDPScissorDoesNotEqualRenderingStateScissor(rcp: Option<&mut RCP>) -> bool {
    let rcp = rcp.unwrap();
    rcp.rdp.rendering_state.scissor != rcp.rdp.scissor
}

#[no_mangle]
pub extern "C" fn RDPGetOtherModeL(rcp: Option<&mut RCP>) -> u32 {
    let rcp = rcp.unwrap();
    rcp.rdp.other_mode_l
}

#[no_mangle]
pub extern "C" fn RDPGetOtherModeH(rcp: Option<&mut RCP>) -> u32 {
    let rcp = rcp.unwrap();
    rcp.rdp.other_mode_h
}

#[no_mangle]
pub extern "C" fn RDPSetOtherModeL(rcp: Option<&mut RCP>, value: u32) {
    let rcp = rcp.unwrap();
    rcp.rdp.other_mode_l = value;
}

#[no_mangle]
pub extern "C" fn RDPSetOtherModeH(rcp: Option<&mut RCP>, value: u32) {
    let rcp = rcp.unwrap();
    rcp.rdp.other_mode_h = value;
}

#[no_mangle]
pub extern "C" fn RDPGetCombineU32(rcp: Option<&mut RCP>) -> u32 {
    let rcp = rcp.unwrap();
    rcp.rdp.combine.to_u32()
}

#[no_mangle]
pub extern "C" fn RDPGetCombine(rcp: Option<&mut RCP>) -> *const CombineParams {
    let rcp = rcp.unwrap();
    Box::into_raw(Box::new(rcp.rdp.combine))
}

#[no_mangle]
pub extern "C" fn RDPSetCombine(rcp: Option<&mut RCP>, value: *mut CombineParams) {
    let rcp = rcp.unwrap();
    rcp.rdp.combine = unsafe { *value };
}

#[no_mangle]
pub extern "C" fn RDPUpdateRenderState(
    rcp: Option<&mut RCP>,
    vertex_id1: u8,
    vertex_id2: u8,
    vertex_id3: u8,
) {
    let rcp = rcp.unwrap();

    let vertex1 = &rcp.rsp.vertex_table[vertex_id1 as usize];
    let vertex2 = &rcp.rsp.vertex_table[vertex_id2 as usize];
    let vertex3 = &rcp.rsp.vertex_table[vertex_id3 as usize];
    let vertex_array = [vertex1, vertex2, vertex3];

    rcp.rdp.update_render_state(
        rcp.gfx_device.as_ref().unwrap(),
        rcp.rsp.geometry_mode,
        &vertex_array,
    );
}
