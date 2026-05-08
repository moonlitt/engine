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

use std::sync::atomic::{AtomicI32, Ordering};
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

/// OR-accumulator for `IComponentHandler::restartComponent` flag bits.
/// The plug-in writes from arbitrary threads; the render path drains on
/// each render and dispatches handling.
pub(crate) type RestartFlags = Arc<AtomicI32>;

pub(crate) struct ComponentHandler {
    queue: ParamQueue,
    restart_flags: RestartFlags,
}

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        crate::trace::emit(&format!("ComponentHandler::beginEdit id={id}"));
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value_normalized: ParamValue) -> tresult {
        crate::trace::emit(&format!(
            "ComponentHandler::performEdit id={id} value={value_normalized}"
        ));
        if let Ok(mut q) = self.queue.lock() {
            q.push(PendingParam { id, value: value_normalized });
        }
        kResultOk
    }

    unsafe fn endEdit(&self, id: ParamID) -> tresult {
        crate::trace::emit(&format!("ComponentHandler::endEdit id={id}"));
        kResultOk
    }

    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        crate::trace::emit(&format!(
            "ComponentHandler::restartComponent flags=0x{flags:08X}"
        ));
        // OR the new flag bits into the shared accumulator. The render
        // path reads-and-clears this on each block and dispatches to
        // whichever flag-specific handler applies.
        self.restart_flags.fetch_or(flags, Ordering::AcqRel);
        kResultOk
    }
}

/// Create a new IComponentHandler COM wrapper paired with its drain queue
/// and restart-flags accumulator.
pub(crate) fn create_component_handler(
) -> (ComWrapper<ComponentHandler>, ParamQueue, RestartFlags) {
    let queue = Arc::new(Mutex::new(Vec::new()));
    let restart_flags = Arc::new(AtomicI32::new(0));
    let handler = ComponentHandler {
        queue: Arc::clone(&queue),
        restart_flags: Arc::clone(&restart_flags),
    };
    (ComWrapper::new(handler), queue, restart_flags)
}

/// Drain all pending parameter changes, returning them in arrival order.
pub(crate) fn drain(queue: &ParamQueue) -> Vec<PendingParam> {
    queue
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

/// Atomically read and clear the accumulated restart flags. Returns 0
/// when no restart was requested since the last drain.
pub(crate) fn drain_restart_flags(flags: &RestartFlags) -> i32 {
    flags.swap(0, Ordering::AcqRel)
}
