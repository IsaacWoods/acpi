pub mod interrupt;
pub mod pci;

pub use interrupt::InterruptModel;
pub use pci::PciConfigRegions;

use crate::{
    AcpiError,
    AcpiHandler,
    AcpiTables,
    PowerProfile,
    address::GenericAddress,
    sdt::{
        Signature,
        fadt::Fadt,
        madt::{Madt, MadtError, MpProtectedModeWakeupCommand, MultiprocessorWakeupMailbox},
    },
};
use alloc::{alloc::Global, vec::Vec};
use core::{alloc::Allocator, mem, ptr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessorState {
    /// A processor in this state is unusable, and you must not attempt to bring it up.
    Disabled,

    /// A processor waiting for a SIPI (Startup Inter-processor Interrupt) is currently not active,
    /// but may be brought up.
    WaitingForSipi,

    /// A Running processor is currently brought up and running code.
    Running,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Processor {
    /// Corresponds to the `_UID` object of the processor's `Device`, or the `ProcessorId` field of the `Processor`
    /// object, in AML.
    pub processor_uid: u32,
    /// The ID of the local APIC of the processor. Will be less than `256` if the APIC is being used, but can be
    /// greater than this if the X2APIC is being used.
    pub local_apic_id: u32,

    /// The state of this processor. Check that the processor is not `Disabled` before attempting to bring it up!
    pub state: ProcessorState,

    /// Whether this processor is the Bootstrap Processor (BSP), or an Application Processor (AP).
    /// When the bootloader is entered, the BSP is the only processor running code. To run code on
    /// more than one processor, you need to "bring up" the APs.
    pub is_ap: bool,
}

#[derive(Debug, Clone)]
pub struct ProcessorInfo<A: Allocator = Global> {
    pub boot_processor: Processor,
    /// Application processors should be brought up in the order they're defined in this list.
    pub application_processors: Vec<Processor, A>,
}

impl<A: Allocator> ProcessorInfo<A> {
    pub(crate) fn new_in(boot_processor: Processor, application_processors: Vec<Processor, A>) -> Self {
        Self { boot_processor, application_processors }
    }
}

/// Information about the ACPI Power Management Timer (ACPI PM Timer).
#[derive(Debug, Clone)]
pub struct PmTimer {
    /// A generic address to the register block of ACPI PM Timer.
    pub base: GenericAddress,
    /// This field is `true` if the hardware supports 32-bit timer, and `false` if the hardware supports 24-bit timer.
    pub supports_32bit: bool,
}

impl PmTimer {
    pub fn new(fadt: &Fadt) -> Result<Option<PmTimer>, AcpiError> {
        match fadt.pm_timer_block()? {
            Some(base) => Ok(Some(PmTimer { base, supports_32bit: { fadt.flags }.pm_timer_is_32_bit() })),
            None => Ok(None),
        }
    }
}

/// `PlatformInfo` allows the collection of some basic information about the platform from some of the fixed-size
/// tables in a nice way. It requires access to the `FADT` and `MADT`. It is the easiest way to get information
/// about the processors and interrupt controllers on a platform.
#[derive(Debug, Clone)]
pub struct PlatformInfo<A: Allocator = Global> {
    pub power_profile: PowerProfile,
    pub interrupt_model: InterruptModel<A>,
    /// On `x86_64` platforms that support the APIC, the processor topology must also be inferred from the
    /// interrupt model. That information is stored here, if present.
    pub processor_info: Option<ProcessorInfo<A>>,
    pub pm_timer: Option<PmTimer>,
    /*
     * TODO: we could provide a nice view of the hardware register blocks in the FADT here.
     */
}

impl PlatformInfo<Global> {
    pub fn new<H>(tables: &AcpiTables<H>) -> Result<Self, AcpiError>
    where
        H: AcpiHandler,
    {
        Self::new_in(tables, alloc::alloc::Global)
    }
}

impl<A: Allocator + Clone> PlatformInfo<A> {
    pub fn new_in<H>(tables: &AcpiTables<H>, allocator: A) -> Result<Self, AcpiError>
    where
        H: AcpiHandler,
    {
        let Some(fadt) = tables.find_table::<Fadt>() else { Err(AcpiError::TableNotFound(Signature::FADT))? };
        let power_profile = fadt.power_profile();

        let (interrupt_model, processor_info) = InterruptModel::new_in(&tables, allocator)?;
        let pm_timer = PmTimer::new(&fadt)?;

        Ok(PlatformInfo { power_profile, interrupt_model, processor_info, pm_timer })
    }
}

/// Wakes up Application Processors (APs) using the Multiprocessor Wakeup Mailbox mechanism.
///
/// For Intel processors, the execution environment is:
/// - Interrupts must be disabled.
/// - RFLAGES.IF set to 0.
/// - Long mode enabled.
/// - Paging mode is enabled and physical memory for waking vector is identity mapped (virtual address equals physical address).
/// - Waking vector must be contained within one physical page.
/// - Selectors are set to flat and otherwise not used.
pub unsafe fn wakeup_aps<H>(
    tables: &AcpiTables<H>,
    handler: H,
    apic_id: u32,
    wakeup_vector: u64,
    timeout_loops: u64,
) -> Result<(), AcpiError>
where
    H: AcpiHandler,
{
    let Some(madt) = tables.find_table::<Madt>() else { Err(AcpiError::TableNotFound(Signature::MADT))? };
    let mailbox_addr = madt.get().get_mpwk_mailbox_addr()?;
    let mut mpwk_mapping = unsafe {
        handler.map_physical_region::<MultiprocessorWakeupMailbox>(
            mailbox_addr as usize,
            mem::size_of::<MultiprocessorWakeupMailbox>(),
        )
    };

    // Reset command
    unsafe {
        ptr::write_volatile(&mut mpwk_mapping.command, MpProtectedModeWakeupCommand::Noop as u16);
    }

    // Fill the mailbox
    mpwk_mapping.apic_id = apic_id;
    mpwk_mapping.wakeup_vector = wakeup_vector;
    unsafe {
        ptr::write_volatile(&mut mpwk_mapping.command, MpProtectedModeWakeupCommand::Wakeup as u16);
    }

    // Wait to join
    let mut loops = 0;
    let mut command = MpProtectedModeWakeupCommand::Wakeup;
    while command != MpProtectedModeWakeupCommand::Noop {
        if loops >= timeout_loops {
            return Err(AcpiError::InvalidMadt(MadtError::WakeupApsTimeout));
        }
        // SAFETY: The caller must ensure that the provided `handler` correctly handles these
        // operations and that the specified `mailbox_addr` is valid.
        unsafe {
            command = ptr::read_volatile(&mpwk_mapping.command).into();
        }
        core::hint::spin_loop();
        loops += 1;
    }
    drop(mpwk_mapping);

    Ok(())
}
