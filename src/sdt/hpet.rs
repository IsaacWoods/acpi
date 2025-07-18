use crate::{
    AcpiError,
    RegionMapper,
    AcpiTable,
    AcpiTables,
    address::RawGenericAddress,
    sdt::{SdtHeader, Signature},
};
use bit_field::BitField;
use log::warn;

#[derive(Debug)]
pub enum PageProtection {
    None,
    /// Access to the rest of the 4KiB, relative to the base address, will not generate a fault.
    Protected4K,
    /// Access to the rest of the 64KiB, relative to the base address, will not generate a fault.
    Protected64K,
    Other,
}

/// Information about the High Precision Event Timer (HPET)
#[derive(Debug)]
pub struct HpetInfo {
    pub hardware_rev: u8,
    pub num_comparators: u8,
    pub main_counter_is_64bits: bool,
    pub legacy_irq_capable: bool,
    pub pci_vendor_id: u16,
    pub base_address: usize,
    pub hpet_number: u8,
    /// The minimum number of clock ticks that can be set without losing interrupts (for timers in Periodic Mode)
    pub clock_tick_unit: u16,
    pub page_protection: PageProtection,
}

impl HpetInfo {
    pub fn new<H>(tables: &AcpiTables<H>) -> Result<HpetInfo, AcpiError>
    where
        H: RegionMapper,
    {
        let Some(hpet) = tables.find_table::<HpetTable>() else { Err(AcpiError::TableNotFound(Signature::HPET))? };

        if hpet.base_address.address_space != 0 {
            warn!("HPET reported as not in system memory; tables invalid?");
        }

        let event_timer_block_id = hpet.event_timer_block_id;
        Ok(HpetInfo {
            hardware_rev: event_timer_block_id.get_bits(0..8) as u8,
            num_comparators: event_timer_block_id.get_bits(8..13) as u8,
            main_counter_is_64bits: event_timer_block_id.get_bit(13),
            legacy_irq_capable: event_timer_block_id.get_bit(15),
            pci_vendor_id: event_timer_block_id.get_bits(16..32) as u16,
            base_address: hpet.base_address.address as usize,
            hpet_number: hpet.hpet_number,
            clock_tick_unit: hpet.clock_tick_unit,
            page_protection: match hpet.page_protection_and_oem.get_bits(0..4) {
                0 => PageProtection::None,
                1 => PageProtection::Protected4K,
                2 => PageProtection::Protected64K,
                3..=15 => PageProtection::Other,
                _ => unreachable!(),
            },
        })
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct HpetTable {
    pub header: SdtHeader,
    pub event_timer_block_id: u32,
    pub base_address: RawGenericAddress,
    pub hpet_number: u8,
    pub clock_tick_unit: u16,
    /// Bits `0..4` specify the page protection guarantee. Bits `4..8` are reserved for OEM attributes.
    pub page_protection_and_oem: u8,
}

unsafe impl AcpiTable for HpetTable {
    const SIGNATURE: Signature = Signature::HPET;

    fn header(&self) -> &SdtHeader {
        &self.header
    }
}
