//! This module defines a trait for each of the possible callbacks which may be implemented for
//! interaction with the jack API Note that these callback handlers do not have thread safety
//! marker constraints because the client always takes ownership of the callback Handlers, ensuring
//! that the callbacks will only be called in a thread safe manner

use types::*;

/// the CallbackContext is passed to some callback handlers and used by some methods to maintain
/// some context and control lifetimes during callbacks
pub struct CallbackContext {}

impl CallbackContext {
    #[doc(hidden)]
    pub fn new() -> Self { CallbackContext { } }
}

/// This trait defines a handler for the process callback
pub trait ProcessHandler {
    fn process(&mut self, ctx: &CallbackContext, nframes: NumFrames) -> i32;
}

/// This trait defines the callbacks which may be delivered to the metadata thread
pub trait MetadataHandler {
    /// Called when the sample rate is changed
    #[allow(unused_variables)]
    fn sample_rate_changed(&mut self, srate: NumFrames) -> i32 { 0 }

    /// Called when ports are connected
    #[allow(unused_variables)]
    fn on_port_connect(&mut self, a: PortId, b: PortId, status: PortConnectStatus) { }

    fn on_xrun(&mut self) -> i32 { 0 }

    /// Function must return all the types of callbacks it wishes to be given
    fn callbacks_of_interest(&self) -> Vec<MetadataHandlers>;
}

pub enum MetadataHandlers {
    SampleRate,
    PortConnect,
    Shutdown,
    Freewheel,
    BufferSize,
    ClientRegistration,
    PortRegistration,
    PortRename,
    GraphOrder,
    Xrun,
}
