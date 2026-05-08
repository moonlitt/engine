//! IComponentHandler implementation.
//!
//! Plugins (specifically their IEditController side) call into this to notify
//! the host when a parameter changes — typically `performEdit(paramId, value)`
//! after the user moves a knob, or programmatic state changes triggered by
//! `setParamNormalized`. The host's job is to forward these into the next
//! audio block via `ProcessData::inputParameterChanges` so the processor
//! actually hears about them.
//!
//! Without this hookup, all controller-side parameter writes are silently
//! dropped — sampler-style plugins (Keyscape, Kontakt, sfizz) never receive
//! their patch selection and stay silent.

use std::sync::{Arc, Mutex};

use vst3::Steinberg::Vst::{
    IComponentHandler, IComponentHandlerTrait, ParamID, ParamValue,
};
use vst3::Steinberg::{int32, kResultOk, tresult};
use vst3::{Class, ComWrapper};

/// One pending controller→processor parameter change.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingParam {
    pub id: ParamID,
    pub value: ParamValue,
}

/// Shared queue between the COM impl (writer, called by the plugin's
/// controller from any thread) and the audio render path (reader/drainer).
pub(crate) type ParamQueue = Arc<Mutex<Vec<PendingParam>>>;

pub(crate) struct ComponentHandler {
    queue: ParamQueue,
}

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value_normalized: ParamValue) -> tresult {
        if let Ok(mut q) = self.queue.lock() {
            q.push(PendingParam { id, value: value_normalized });
        }
        kResultOk
    }

    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn restartComponent(&self, _flags: int32) -> tresult {
        // Host accepts the request but does not act on it — full hot-restart
        // (sample rate / bus reconfiguration) is not yet supported.
        kResultOk
    }
}

/// Create a new IComponentHandler COM wrapper paired with its drain queue.
/// Caller passes the wrapper to `IEditController::setComponentHandler` and
/// keeps the queue handle to drain pending edits before each `process()`.
pub(crate) fn create_component_handler() -> (ComWrapper<ComponentHandler>, ParamQueue) {
    let queue = Arc::new(Mutex::new(Vec::new()));
    let handler = ComponentHandler { queue: Arc::clone(&queue) };
    (ComWrapper::new(handler), queue)
}

/// Drain all pending parameter changes, returning them in arrival order.
pub(crate) fn drain(queue: &ParamQueue) -> Vec<PendingParam> {
    queue
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}
