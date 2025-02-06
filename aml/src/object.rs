use alloc::{sync::Arc, vec::Vec};

#[derive(Debug)]
pub enum Object {
    Uninitialized,
    Buffer(Vec<u8>),
    BufferField,
    Device,
    Event,
    FieldUnit,
    Integer(u64),
    Method,
    Mutex,
    Reference(Arc<Object>),
    OpRegion,
    Package,
    PowerResource,
    Processor,
    RawDataBuffer,
    String,
    ThermalZone,
}

impl Object {
    /*
     * TODO XXX: this is a horrendous hack to emulate a clever locking solution for dynamically
     * validating borrow checking for objects at A Later Date. It is trivially easy to produce
     * undefined behaviour with this (and might be UB intrinsically).
     *
     * Options are:
     *   - Put something like an AtomicRefCell around every single object. This is too slow I
     *     think.
     *   - Utilise a global lock on the namespace that gives us some sort of token we can then
     *     magic up mutable references to objects through. Safety is ensured at type-level.
     *   - Something else cleverer.
     */
    pub fn gain_mut(&self) -> &mut Self {
        #[allow(invalid_reference_casting)]
        unsafe {
            &mut *(self as *const Self as *mut Self)
        }
    }
}
