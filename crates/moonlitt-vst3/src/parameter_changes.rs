//! IParameterChanges + IParamValueQueue COM implementations.
//!
//! These are the input side of the controller→processor parameter forwarding
//! path. The host owns the implementations; the plugin reads from them through
//! `ProcessData::inputParameterChanges`.
//!
//! Built fresh on each render() call from the drained ComponentHandler queue,
//! so they live for exactly one process() invocation. Plugins are not allowed
//! to mutate input changes (per VST3 spec), so addPoint / addParameterData
//! are no-ops.

use std::collections::HashMap;

use vst3::Steinberg::Vst::{
    IParamValueQueue, IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID,
    ParamValue,
};
use vst3::Steinberg::{int32, kResultFalse, kResultOk, tresult};
use vst3::{Class, ComWrapper};

use crate::component_handler::PendingParam;

pub(crate) struct ParamValueQueueImpl {
    id: ParamID,
    points: Vec<(int32, ParamValue)>,
}

impl Class for ParamValueQueueImpl {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamValueQueueImpl {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.points.len() as int32
    }

    unsafe fn getPoint(
        &self,
        index: int32,
        sample_offset: *mut int32,
        value: *mut ParamValue,
    ) -> tresult {
        match self.points.get(index as usize) {
            Some(&(off, val)) => {
                std::ptr::write(sample_offset, off);
                std::ptr::write(value, val);
                kResultOk
            }
            None => kResultFalse,
        }
    }

    unsafe fn addPoint(
        &self,
        _sample_offset: int32,
        _value: ParamValue,
        _index: *mut int32,
    ) -> tresult {
        // Input parameter changes are read-only from the plugin's view.
        kResultFalse
    }
}

pub(crate) struct ParameterChangesImpl {
    queues: Vec<ComWrapper<ParamValueQueueImpl>>,
}

impl Class for ParameterChangesImpl {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for ParameterChangesImpl {
    unsafe fn getParameterCount(&self) -> int32 {
        self.queues.len() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        self.queues
            .get(index as usize)
            .and_then(|q| q.as_com_ref::<IParamValueQueue>())
            .map(|r| r.as_ptr())
            .unwrap_or(std::ptr::null_mut())
    }

    unsafe fn addParameterData(
        &self,
        _id: *const ParamID,
        _index: *mut int32,
    ) -> *mut IParamValueQueue {
        // Read-only on input changes.
        std::ptr::null_mut()
    }
}

/// Build a ParameterChanges COM wrapper from drained pending edits. Multiple
/// edits to the same paramID coalesce into a single queue with all points
/// preserved in arrival order.
pub(crate) fn build_input_changes(
    pending: &[PendingParam],
) -> Option<ComWrapper<ParameterChangesImpl>> {
    if pending.is_empty() {
        return None;
    }

    let mut by_id: HashMap<ParamID, Vec<(int32, ParamValue)>> = HashMap::new();
    for p in pending {
        // sampleOffset 0 means "apply at start of block" — close enough; we
        // don't currently sub-sample-accurate edits.
        by_id.entry(p.id).or_default().push((0, p.value));
    }

    let queues = by_id
        .into_iter()
        .map(|(id, points)| ComWrapper::new(ParamValueQueueImpl { id, points }))
        .collect();

    Some(ComWrapper::new(ParameterChangesImpl { queues }))
}
