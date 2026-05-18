use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::debug::DebugSource;
use crate::debug::Debuggable;
use crate::debug::Snapshottable;
use crate::debug::event::MemoryDebugEvent;
use crate::debug::view::MemoryDebugView;
use crate::system::clock::MasterClockTick;
use crate::system::CpcModel;

pub trait MemRead {
    fn read_byte(&self, address: usize) -> u8;

    fn read_word(&self, address: usize) -> u16 {
        let low_byte = self.read_byte(address);
        let high_byte = self.read_byte(address + 1);
        u16::from_le_bytes([low_byte, high_byte])
    }
}

pub trait MemWrite {
    fn write_byte(&mut self, address: usize, value: u8);

    fn write_word(&mut self, address: usize, value: u16) {
        let bytes = value.to_le_bytes();
        self.write_byte(address, bytes[0]);
        self.write_byte(address + 1, bytes[1]);
    }
}

pub trait MemManage {
    fn enable_lower_rom(&mut self, enable: bool);

    fn enable_upper_rom(&mut self, enable: bool);

    fn select_upper_rom(&mut self, upper_rom_nr: u8);

    fn force_ram_read(&mut self, force: bool);

    // Default no-op: only the CPC 6128 implements expansion RAM banking.
    fn set_ram_config(&mut self, _config: u8) {}
}

#[derive(Serialize, Deserialize)]
pub struct Rom {
    #[serde(rename = "rom")]
    data: Vec<u8>,
    address_mask: usize,
    master_clock: MasterClockTick,
}

impl Rom {
    pub fn step(&mut self, master_clock: MasterClockTick) {
        self.master_clock = master_clock;
    }

    pub fn from_bytes(bytes: &[u8]) -> Rom {
        // TODO: better error handling
        // TODO: check ROM size (should be 16k)
        let data = bytes.to_vec();
        debug_assert!(data.len().is_power_of_two());
        let address_mask = data.len() - 1;
        let master_clock = MasterClockTick::default();

        Rom {
            data,
            address_mask,
            master_clock,
        }
    }
}

impl MemRead for Rom {
    fn read_byte(&self, address: usize) -> u8 {
        let address = address & self.address_mask;
        let value = self.data[address];
        self.emit_debug_event(
            MemoryDebugEvent::MemoryRead { address, value },
            self.master_clock,
        );

        value
    }
}

pub struct RomDebugView {
    data: Vec<u8>,
}

impl Snapshottable for Rom {
    type View = RomDebugView;

    fn debug_view(&self) -> Self::View {
        Self::View {
            data: self.data.clone(),
        }
    }
}

impl Debuggable for Rom {
    const SOURCE: DebugSource = DebugSource::Memory;
    type Event = MemoryDebugEvent;
}

#[derive(Serialize, Deserialize)]
pub struct Ram {
    #[serde(rename = "ram")]
    data: Vec<u8>,
    address_mask: usize,
    master_clock: MasterClockTick,
}

impl Ram {
    pub fn new(size: usize) -> Ram {
        let data = vec![0; size];
        debug_assert!(data.len().is_power_of_two());
        let address_mask = data.len() - 1;
        let master_clock = MasterClockTick::default();

        Ram {
            data,
            address_mask,
            master_clock,
        }
    }

    pub fn from_bytes(size: usize, bytes: &[u8], offset: usize) -> Ram {
        // TODO: better error handling
        // TODO: check if ROM fits
        let mut ram = Ram::new(size);

        for (i, byte) in bytes.iter().enumerate() {
            ram.write_byte(offset + i, *byte);
        }

        ram
    }

    pub fn step(&mut self, master_clock: MasterClockTick) {
        self.master_clock = master_clock;
    }
}

impl MemRead for Ram {
    fn read_byte(&self, address: usize) -> u8 {
        let address = address & self.address_mask;
        let value = self.data[address];
        self.emit_debug_event(
            MemoryDebugEvent::MemoryRead { address, value },
            self.master_clock,
        );

        value
    }
}

impl MemWrite for Ram {
    fn write_byte(&mut self, address: usize, value: u8) {
        let address = address & self.address_mask;
        let was = self.data[address];
        self.data[address] = value;
        self.emit_debug_event(
            MemoryDebugEvent::MemoryWritten {
                address,
                is: value,
                was,
            },
            self.master_clock,
        );
    }
}

// TODO: can we get rid of this empty impl? Currently required for bus writes.
impl MemManage for Ram {
    fn enable_lower_rom(&mut self, _enable: bool) {}

    fn enable_upper_rom(&mut self, _enable: bool) {}

    fn select_upper_rom(&mut self, _upper_rom_nr: u8) {}

    fn force_ram_read(&mut self, _force: bool) {}
}

pub struct RamDebugView {
    data: Vec<u8>,
}

impl Snapshottable for Ram {
    type View = RamDebugView;

    fn debug_view(&self) -> Self::View {
        Self::View {
            data: self.data.clone(),
        }
    }
}

impl Debuggable for Ram {
    const SOURCE: DebugSource = DebugSource::Memory;
    type Event = MemoryDebugEvent;
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCpcX64 {
    #[serde(flatten)]
    ram: Ram,
    #[serde(flatten)]
    lower_rom: Rom,
    lower_rom_enabled: bool,
    upper_roms: HashMap<u8, Rom>,
    selected_upper_rom: u8,
    upper_rom_enabled: bool,
    ram_read_forced: bool,
}

impl MemoryCpcX64 {
    pub fn new(model: CpcModel) -> Self {
        let (lower_rom, basic_rom): (&[u8], &[u8]) = match model {
            CpcModel::Cpc464 => (
                include_bytes!("../../rom/os_464.rom"),
                include_bytes!("../../rom/basic_1.0.rom"),
            ),
            CpcModel::Cpc664 => (
                include_bytes!("../../rom/os_664.rom"),
                include_bytes!("../../rom/basic_664.rom"),
            ),
            CpcModel::Cpc6128 => (
                include_bytes!("../../rom/os_6128.rom"),
                include_bytes!("../../rom/basic_6128.rom"),
            ),
        };

        let mut upper_roms = HashMap::new();
        upper_roms.insert(0, Rom::from_bytes(basic_rom));
        upper_roms.insert(
            7,
            Rom::from_bytes(include_bytes!("../../rom/amsdos.rom")),
        );

        MemoryCpcX64 {
            ram: Ram::new(0x10000),
            lower_rom: Rom::from_bytes(lower_rom),
            lower_rom_enabled: true,
            upper_roms,
            selected_upper_rom: 0,
            upper_rom_enabled: true,
            ram_read_forced: false,
        }
    }
}

impl Default for MemoryCpcX64 {
    fn default() -> Self {
        Self::new(CpcModel::Cpc464)
    }
}

impl MemRead for MemoryCpcX64 {
    fn read_byte(&self, address: usize) -> u8 {
        if self.ram_read_forced {
            return self.ram.read_byte(address);
        }

        // TODO: define proper constants
        if self.lower_rom_enabled && address < 0x4000 {
            return self.lower_rom.read_byte(address);
        }

        if self.upper_rom_enabled && address >= 0xc000 {
            match self.upper_roms.get(&self.selected_upper_rom) {
                Some(upper_rom) => {
                    return upper_rom.read_byte(address - 0xc000);
                }
                None => {
                    return self.ram.read_byte(address);
                }
            }
        }

        self.ram.read_byte(address)
    }
}

impl MemWrite for MemoryCpcX64 {
    fn write_byte(&mut self, address: usize, value: u8) {
        self.ram.write_byte(address, value);
    }
}

impl MemManage for MemoryCpcX64 {
    fn enable_lower_rom(&mut self, enable: bool) {
        self.lower_rom_enabled = enable;
    }

    fn enable_upper_rom(&mut self, enable: bool) {
        self.upper_rom_enabled = enable;
    }

    fn select_upper_rom(&mut self, upper_rom_nr: u8) {
        self.selected_upper_rom = upper_rom_nr;
    }

    fn force_ram_read(&mut self, force: bool) {
        self.ram_read_forced = force;
    }
}

impl Snapshottable for MemoryCpcX64 {
    type View = MemoryDebugView;

    fn debug_view(&self) -> Self::View {
        let mut upper_roms = HashMap::new();
        for (key, rom) in &self.upper_roms {
            upper_roms.insert(*key, rom.debug_view().data);
        }

        let ram = self.ram.debug_view().data;
        let ram_extension = vec![];
        let lower_rom = self.lower_rom.debug_view().data;
        let lower_rom_enabled = self.lower_rom_enabled;
        let selected_upper_rom = self.selected_upper_rom;
        let upper_rom_enabled = self.upper_rom_enabled;

        // Create composite RAM view first (just RAM for now, extension RAM not implemented yet)
        let composite_ram = ram.clone();

        // Create composite ROM/RAM view based on composite_ram
        let mut composite_rom_ram = composite_ram.clone();

        if lower_rom_enabled {
            composite_rom_ram[0x0000..0x4000].copy_from_slice(&lower_rom);
        }

        if upper_rom_enabled {
            if let Some(upper_rom_data) = upper_roms.get(&selected_upper_rom) {
                composite_rom_ram[0xC000..0x10000].copy_from_slice(upper_rom_data);
            }
        }

        MemoryDebugView {
            ram,
            ram_extension,
            lower_rom,
            lower_rom_enabled,
            upper_roms,
            selected_upper_rom,
            upper_rom_enabled,
            composite_rom_ram,
            composite_ram,
        }
    }
}

impl Debuggable for MemoryCpcX64 {
    const SOURCE: DebugSource = DebugSource::Memory;
    type Event = MemoryDebugEvent;
}

// CPC 6128 RAM configuration table. Rows index by config (0..=7); each entry
// names the physical 16K bank visible at the corresponding 16K CPU slot
// (slot 0 = 0x0000-0x3FFF, slot 1 = 0x4000-0x7FFF, slot 2 = 0x8000-0xBFFF,
// slot 3 = 0xC000-0xFFFF). Standard mapping per CPCWIKI / Grimware docs.
const RAM_BANK_SIZE: usize = 0x4000;
const RAM_CONFIG_TABLE: [[usize; 4]; 8] = [
    [0, 1, 2, 3],
    [0, 1, 2, 7],
    [4, 5, 6, 7],
    [0, 3, 2, 7],
    [0, 4, 2, 3],
    [0, 5, 2, 3],
    [0, 6, 2, 3],
    [0, 7, 2, 3],
];

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCpc6128 {
    #[serde(flatten)]
    ram: Ram, // 128K, eight 16K banks
    #[serde(flatten)]
    lower_rom: Rom,
    lower_rom_enabled: bool,
    upper_roms: HashMap<u8, Rom>,
    selected_upper_rom: u8,
    upper_rom_enabled: bool,
    ram_read_forced: bool,
    ram_config: u8,
}

impl MemoryCpc6128 {
    pub fn new() -> Self {
        let mut upper_roms = HashMap::new();
        upper_roms.insert(
            0,
            Rom::from_bytes(include_bytes!("../../rom/basic_6128.rom")),
        );
        upper_roms.insert(7, Rom::from_bytes(include_bytes!("../../rom/amsdos.rom")));

        MemoryCpc6128 {
            ram: Ram::new(0x20000),
            lower_rom: Rom::from_bytes(include_bytes!("../../rom/os_6128.rom")),
            lower_rom_enabled: true,
            upper_roms,
            selected_upper_rom: 0,
            upper_rom_enabled: true,
            ram_read_forced: false,
            ram_config: 0,
        }
    }

    fn resolve_ram_address(&self, address: usize) -> usize {
        let slot = (address >> 14) & 0x3;
        let offset = address & 0x3FFF;
        let bank = RAM_CONFIG_TABLE[(self.ram_config & 0x7) as usize][slot];
        bank * RAM_BANK_SIZE + offset
    }
}

impl Default for MemoryCpc6128 {
    fn default() -> Self {
        Self::new()
    }
}

impl MemRead for MemoryCpc6128 {
    fn read_byte(&self, address: usize) -> u8 {
        if self.ram_read_forced {
            return self.ram.read_byte(self.resolve_ram_address(address));
        }

        if self.lower_rom_enabled && address < 0x4000 {
            return self.lower_rom.read_byte(address);
        }

        if self.upper_rom_enabled && address >= 0xc000 {
            match self.upper_roms.get(&self.selected_upper_rom) {
                Some(upper_rom) => return upper_rom.read_byte(address - 0xc000),
                None => return self.ram.read_byte(self.resolve_ram_address(address)),
            }
        }

        self.ram.read_byte(self.resolve_ram_address(address))
    }
}

impl MemWrite for MemoryCpc6128 {
    fn write_byte(&mut self, address: usize, value: u8) {
        let phys = self.resolve_ram_address(address);
        self.ram.write_byte(phys, value);
    }
}

impl MemManage for MemoryCpc6128 {
    fn enable_lower_rom(&mut self, enable: bool) {
        self.lower_rom_enabled = enable;
    }

    fn enable_upper_rom(&mut self, enable: bool) {
        self.upper_rom_enabled = enable;
    }

    fn select_upper_rom(&mut self, upper_rom_nr: u8) {
        self.selected_upper_rom = upper_rom_nr;
    }

    fn force_ram_read(&mut self, force: bool) {
        self.ram_read_forced = force;
    }

    fn set_ram_config(&mut self, config: u8) {
        self.ram_config = config & 0x7;
    }
}

impl Snapshottable for MemoryCpc6128 {
    type View = MemoryDebugView;

    fn debug_view(&self) -> Self::View {
        let mut upper_roms = HashMap::new();
        for (key, rom) in &self.upper_roms {
            upper_roms.insert(*key, rom.debug_view().data);
        }

        let all_ram = self.ram.debug_view().data;
        let ram = all_ram[..0x10000].to_vec();
        let ram_extension = all_ram[0x10000..0x20000].to_vec();
        let lower_rom = self.lower_rom.debug_view().data;
        let lower_rom_enabled = self.lower_rom_enabled;
        let selected_upper_rom = self.selected_upper_rom;
        let upper_rom_enabled = self.upper_rom_enabled;

        // Build the 64K composite RAM by walking the current bank mapping.
        let mapping = &RAM_CONFIG_TABLE[(self.ram_config & 0x7) as usize];
        let mut composite_ram = vec![0u8; 0x10000];
        for (slot, &bank) in mapping.iter().enumerate() {
            let src = bank * RAM_BANK_SIZE;
            let dst = slot * RAM_BANK_SIZE;
            composite_ram[dst..dst + RAM_BANK_SIZE]
                .copy_from_slice(&all_ram[src..src + RAM_BANK_SIZE]);
        }

        let mut composite_rom_ram = composite_ram.clone();

        if lower_rom_enabled {
            composite_rom_ram[0x0000..0x4000].copy_from_slice(&lower_rom);
        }

        if upper_rom_enabled {
            if let Some(upper_rom_data) = upper_roms.get(&selected_upper_rom) {
                composite_rom_ram[0xC000..0x10000].copy_from_slice(upper_rom_data);
            }
        }

        MemoryDebugView {
            ram,
            ram_extension,
            lower_rom,
            lower_rom_enabled,
            upper_roms,
            selected_upper_rom,
            upper_rom_enabled,
            composite_rom_ram,
            composite_ram,
        }
    }
}

impl Debuggable for MemoryCpc6128 {
    const SOURCE: DebugSource = DebugSource::Memory;
    type Event = MemoryDebugEvent;
}

#[derive(Serialize, Deserialize)]
pub enum AnyMemory {
    CpcX64(MemoryCpcX64),
    Cpc6128(MemoryCpc6128),
}

impl Default for AnyMemory {
    fn default() -> Self {
        AnyMemory::CpcX64(MemoryCpcX64::default())
    }
}

impl MemRead for AnyMemory {
    fn read_byte(&self, address: usize) -> u8 {
        match self {
            AnyMemory::CpcX64(memory) => memory.read_byte(address),
            AnyMemory::Cpc6128(memory) => memory.read_byte(address),
        }
    }

    fn read_word(&self, address: usize) -> u16 {
        match self {
            AnyMemory::CpcX64(memory) => memory.read_word(address),
            AnyMemory::Cpc6128(memory) => memory.read_word(address),
        }
    }
}

impl MemWrite for AnyMemory {
    fn write_byte(&mut self, address: usize, value: u8) {
        match self {
            AnyMemory::CpcX64(memory) => memory.write_byte(address, value),
            AnyMemory::Cpc6128(memory) => memory.write_byte(address, value),
        }
    }

    fn write_word(&mut self, address: usize, value: u16) {
        match self {
            AnyMemory::CpcX64(memory) => memory.write_word(address, value),
            AnyMemory::Cpc6128(memory) => memory.write_word(address, value),
        }
    }
}

impl MemManage for AnyMemory {
    fn enable_lower_rom(&mut self, enable: bool) {
        match self {
            AnyMemory::CpcX64(memory) => memory.enable_lower_rom(enable),
            AnyMemory::Cpc6128(memory) => memory.enable_lower_rom(enable),
        }
    }

    fn enable_upper_rom(&mut self, enable: bool) {
        match self {
            AnyMemory::CpcX64(memory) => memory.enable_upper_rom(enable),
            AnyMemory::Cpc6128(memory) => memory.enable_upper_rom(enable),
        }
    }

    fn select_upper_rom(&mut self, upper_rom_nr: u8) {
        match self {
            AnyMemory::CpcX64(memory) => memory.select_upper_rom(upper_rom_nr),
            AnyMemory::Cpc6128(memory) => memory.select_upper_rom(upper_rom_nr),
        }
    }

    fn force_ram_read(&mut self, force: bool) {
        match self {
            AnyMemory::CpcX64(memory) => memory.force_ram_read(force),
            AnyMemory::Cpc6128(memory) => memory.force_ram_read(force),
        }
    }

    fn set_ram_config(&mut self, config: u8) {
        match self {
            AnyMemory::CpcX64(memory) => memory.set_ram_config(config),
            AnyMemory::Cpc6128(memory) => memory.set_ram_config(config),
        }
    }
}

impl Snapshottable for AnyMemory {
    type View = MemoryDebugView;

    fn debug_view(&self) -> Self::View {
        match self {
            AnyMemory::CpcX64(memory) => memory.debug_view(),
            AnyMemory::Cpc6128(memory) => memory.debug_view(),
        }
    }
}

#[cfg(test)]
/// A test memory implementation that returns predefined values and ignores writes
#[derive(Default)]
pub struct TestMemory {
    read_values: Vec<u8>,
    current_index: std::cell::Cell<usize>,
}

#[cfg(test)]
impl TestMemory {
    pub fn new(read_values: Vec<u8>) -> Self {
        use std::cell::Cell;

        Self {
            read_values,
            current_index: Cell::new(0),
        }
    }
}

#[cfg(test)]
impl MemRead for TestMemory {
    fn read_byte(&self, _address: usize) -> u8 {
        let current_index = self.current_index.get();
        if current_index < self.read_values.len() {
            let value = self.read_values[current_index];
            self.current_index.set(current_index + 1);
            value
        } else {
            0x00 // Default value when no more data
        }
    }
}

#[cfg(test)]
impl MemWrite for TestMemory {
    fn write_byte(&mut self, _address: usize, _value: u8) {
        // Noop - writes are ignored in test memory
    }
}

#[cfg(test)]
impl MemManage for TestMemory {
    fn enable_lower_rom(&mut self, _enable: bool) {
        // Noop
    }

    fn enable_upper_rom(&mut self, _enable: bool) {
        // Noop
    }

    fn select_upper_rom(&mut self, _upper_rom_nr: u8) {
        // Noop
    }

    fn force_ram_read(&mut self, _force: bool) {
        // Noop
    }
}

#[cfg(test)]
mod cpc6128_banking_tests {
    use super::*;

    fn fresh() -> MemoryCpc6128 {
        let mut mem = MemoryCpc6128::new();
        // Suppress ROM overlays so writes/reads exercise the banking only.
        mem.lower_rom_enabled = false;
        mem.upper_rom_enabled = false;
        mem
    }

    #[test]
    fn config_0_maps_banks_0_to_3() {
        let mut mem = fresh();
        // Write sentinel into each slot under config 0.
        for slot in 0..4 {
            let addr = slot * 0x4000;
            mem.write_byte(addr, (0xA0 + slot) as u8);
        }
        for slot in 0..4 {
            let addr = slot * 0x4000;
            assert_eq!(mem.read_byte(addr), (0xA0 + slot) as u8);
        }
    }

    #[test]
    fn config_2_maps_banks_4_to_7() {
        let mut mem = fresh();
        // Write sentinel into each slot under config 0 (touches banks 0..=3).
        for slot in 0..4 {
            mem.write_byte(slot * 0x4000, (0xA0 + slot) as u8);
        }
        // Switch to config 2 (banks 4,5,6,7) and write distinct sentinels.
        mem.set_ram_config(2);
        for slot in 0..4 {
            mem.write_byte(slot * 0x4000, (0xB0 + slot) as u8);
        }
        // Each slot should read back the config-2 sentinel.
        for slot in 0..4 {
            assert_eq!(mem.read_byte(slot * 0x4000), (0xB0 + slot) as u8);
        }
        // Switch back: banks 0..=3 must still hold the original sentinels.
        mem.set_ram_config(0);
        for slot in 0..4 {
            assert_eq!(mem.read_byte(slot * 0x4000), (0xA0 + slot) as u8);
        }
    }

    #[test]
    fn config_1_swaps_slot_3_for_bank_7() {
        let mut mem = fresh();
        // Under config 0 slot 3 is bank 3. Write A there.
        mem.set_ram_config(0);
        mem.write_byte(0xC000, 0xAA);
        // Under config 1 slot 3 is bank 7. Write B there.
        mem.set_ram_config(1);
        mem.write_byte(0xC000, 0xBB);
        assert_eq!(mem.read_byte(0xC000), 0xBB);
        // Config 0: bank 3 sentinel survives.
        mem.set_ram_config(0);
        assert_eq!(mem.read_byte(0xC000), 0xAA);
        // Config 2: slot 3 is also bank 7 -> same B sentinel.
        mem.set_ram_config(2);
        assert_eq!(mem.read_byte(0xC000), 0xBB);
    }

    #[test]
    fn slots_0_and_2_stay_on_banks_0_and_2_for_configs_3_to_7() {
        let mut mem = fresh();
        mem.set_ram_config(0);
        mem.write_byte(0x0000, 0x11);
        mem.write_byte(0x8000, 0x22);
        for config in 3..=7 {
            mem.set_ram_config(config);
            assert_eq!(mem.read_byte(0x0000), 0x11, "config {config} slot 0");
            assert_eq!(mem.read_byte(0x8000), 0x22, "config {config} slot 2");
        }
    }

    #[test]
    fn ram_config_high_bits_are_masked() {
        let mut mem = fresh();
        // Bits above 0..=2 should not alter the RAM mapping.
        mem.write_byte(0xC000, 0x77);
        mem.set_ram_config(0b1111_1000); // config index = 0
        assert_eq!(mem.read_byte(0xC000), 0x77);
    }
}
