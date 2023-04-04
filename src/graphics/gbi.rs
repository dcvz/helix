use self::defines::Gfx;
use super::{rdp::RDP, rsp::RSP};
use std::collections::HashMap;

pub mod defines;
mod f3dex2;
mod f3dzex2;
mod utils;

pub enum GBIResult {
    Continue,
    Decrease,
    Increase,
    SetAddressWithDecrease(usize),
    Recurse(*const Gfx),
    Return,
}

pub type GBICommand = fn(dp: &mut RDP, rsp: &mut RSP, command: *const Gfx) -> GBIResult;

pub struct GBI {
    pub gbi_opcode_table: HashMap<usize, GBICommand>,
}

trait GBIDefinition {
    fn setup(gbi: &mut GBI);
}

impl GBI {
    pub fn new() -> Self {
        Self {
            gbi_opcode_table: HashMap::new(),
        }
    }

    pub fn setup(&mut self) {
        // TODO: Register some base handlers?

        if cfg!(feature = "f3dzex2") {
            f3dzex2::F3DZEX2::setup(self);
        } else if cfg!(feature = "f3dex2") {
            f3dex2::F3DEX2::setup(self);
        }
    }

    pub fn register(&mut self, opcode: usize, cmd: GBICommand) {
        self.gbi_opcode_table.insert(opcode, cmd);
    }

    pub fn handle_command(&self, rdp: &mut RDP, rsp: &mut RSP, command: *const Gfx) -> GBIResult {
        let opcode = unsafe { (*command).words.w0 } >> 24;
        let cmd = self.gbi_opcode_table.get(&opcode);

        match cmd {
            Some(cmd) => cmd(rdp, rsp, command),
            None => panic!("Unknown GBI opcode: {}", opcode),
        }
    }
}