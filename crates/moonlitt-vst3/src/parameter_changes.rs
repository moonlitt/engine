//! IParameterChanges + IParamValueQueue COM implementations.
//!
//! Two flavors:
//!
//! - **Input** (host writes / plugin reads): the controller→processor
//!   forwarding path, attached to `ProcessData::inputParameterChanges`.
//!   Built fresh from the drained ComponentHandler queue; addPoint /
//!   addParameterData are no-ops (per spec, plugins must not mutate input).
//!
//! - **Output** (plugin writes / host reads): the processor→controller
//!   feedback path, attached to `ProcessData::outputParameterChanges`. The
//!   plugin calls addParameterData / addPoint to report parameter values it
//!   computed during process() (envelope followers, LFO outputs, internal
//!   parameter automation). The host drains these after process() returns
//!   and forwards them to the controller via setParamNormalized so the
//!   controller-side state stays consistent.

use std::cell::UnsafeCell;
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
/// preserved in arrival order. Each point keeps the originator's sample
/// offset so the plug-in can apply sample-accurate parameter automation
/// within a block (e.g. snapping at a specific bar position).
pub(crate) fn build_input_changes(
    pending: &[PendingParam],
) -> Option<ComWrapper<ParameterChangesImpl>> {
    if pending.is_empty() {
        return None;
    }

    let mut by_id: HashMap<ParamID, Vec<(int32, ParamValue)>> = HashMap::new();
    for p in pending {
        by_id
            .entry(p.id)
            .or_default()
            .push((p.sample_offset, p.value));
    }

    let queues = by_id
        .into_iter()
        .map(|(id, points)| ComWrapper::new(ParamValueQueueImpl { id, points }))
        .collect();

    Some(ComWrapper::new(ParameterChangesImpl { queues }))
}

#[cfg(test)]
mod sample_offset_tests {
    use super::*;

    fn read_point(q: &ComWrapper<ParamValueQueueImpl>, index: int32) -> (int32, ParamValue) {
        let mut off: int32 = -1;
        let mut val: ParamValue = -1.0;
        unsafe {
            let r = q.getPoint(index, &mut off as *mut _, &mut val as *mut _);
            assert_eq!(r, kResultOk, "getPoint failed");
        }
        (off, val)
    }

    #[test]
    fn build_input_changes_preserves_sample_offsets() {
        let pending = vec![
            PendingParam {
                id: 42,
                value: 0.25,
                sample_offset: 0,
            },
            PendingParam {
                id: 42,
                value: 0.75,
                sample_offset: 128,
            },
            PendingParam {
                id: 42,
                value: 1.0,
                sample_offset: 255,
            },
        ];
        let changes = build_input_changes(&pending).expect("non-empty pending");
        unsafe {
            assert_eq!(changes.getParameterCount(), 1);
        }
        // Extract the queue and read points in order.
        let queue = unsafe {
            let raw = changes.getParameterData(0);
            assert!(!raw.is_null());
            raw
        };
        unsafe {
            assert_eq!((*queue).vtbl.is_null(), false);
        }

        // Read points via the COM interface to mirror what a plug-in sees.
        unsafe {
            let n = ((*(*queue).vtbl).getPointCount)(queue);
            assert_eq!(n, 3, "all three points should survive");
            let mut points = Vec::new();
            for i in 0..n {
                let mut off: int32 = -1;
                let mut val: ParamValue = -1.0;
                let r = ((*(*queue).vtbl).getPoint)(queue, i, &mut off, &mut val);
                assert_eq!(r, kResultOk);
                points.push((off, val));
            }
            assert_eq!(points, vec![(0, 0.25), (128, 0.75), (255, 1.0)]);
        }
    }

    #[test]
    fn build_input_changes_groups_by_param_id() {
        let pending = vec![
            PendingParam {
                id: 1,
                value: 0.5,
                sample_offset: 0,
            },
            PendingParam {
                id: 2,
                value: 0.5,
                sample_offset: 0,
            },
            PendingParam {
                id: 1,
                value: 0.6,
                sample_offset: 64,
            },
        ];
        let changes = build_input_changes(&pending).unwrap();
        unsafe {
            assert_eq!(changes.getParameterCount(), 2, "two distinct paramIDs");
        }
    }

    #[test]
    fn empty_pending_returns_none() {
        assert!(build_input_changes(&[]).is_none());
    }

    /// Smoke test the helper that mirrors how we read points back.
    #[test]
    fn read_point_returns_offset_and_value() {
        let q = ComWrapper::new(ParamValueQueueImpl {
            id: 7,
            points: vec![(64, 0.5)],
        });
        let (off, val) = read_point(&q, 0);
        assert_eq!((off, val), (64, 0.5));
    }
}

// ---------------------------------------------------------------------------
// Output (plugin writes, host reads)
// ---------------------------------------------------------------------------

pub(crate) struct OutputParamValueQueueImpl {
    id: ParamID,
    points: UnsafeCell<Vec<(int32, ParamValue)>>,
}

impl Class for OutputParamValueQueueImpl {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for OutputParamValueQueueImpl {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id
    }

    unsafe fn getPointCount(&self) -> int32 {
        (*self.points.get()).len() as int32
    }

    unsafe fn getPoint(
        &self,
        index: int32,
        sample_offset: *mut int32,
        value: *mut ParamValue,
    ) -> tresult {
        let pts = &*self.points.get();
        match pts.get(index as usize) {
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
        sample_offset: int32,
        value: ParamValue,
        index: *mut int32,
    ) -> tresult {
        let pts = &mut *self.points.get();
        let idx = pts.len();
        pts.push((sample_offset, value));
        if !index.is_null() {
            std::ptr::write(index, idx as int32);
        }
        kResultOk
    }
}

pub(crate) struct OutputParameterChangesImpl {
    queues: UnsafeCell<Vec<ComWrapper<OutputParamValueQueueImpl>>>,
}

impl Class for OutputParameterChangesImpl {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for OutputParameterChangesImpl {
    unsafe fn getParameterCount(&self) -> int32 {
        (*self.queues.get()).len() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        let queues = &*self.queues.get();
        queues
            .get(index as usize)
            .and_then(|q| q.as_com_ref::<IParamValueQueue>())
            .map(|r| r.as_ptr())
            .unwrap_or(std::ptr::null_mut())
    }

    unsafe fn addParameterData(
        &self,
        id: *const ParamID,
        index: *mut int32,
    ) -> *mut IParamValueQueue {
        let id = *id;
        let queues = &mut *self.queues.get();

        // Reuse an existing queue for this paramID if present.
        for (i, q) in queues.iter().enumerate() {
            if q.as_com_ref::<IParamValueQueue>()
                .map(|r| r.getParameterId() == id)
                .unwrap_or(false)
            {
                if !index.is_null() {
                    std::ptr::write(index, i as int32);
                }
                return q
                    .as_com_ref::<IParamValueQueue>()
                    .map(|r| r.as_ptr())
                    .unwrap_or(std::ptr::null_mut());
            }
        }

        // New queue.
        let new_q = ComWrapper::new(OutputParamValueQueueImpl {
            id,
            points: UnsafeCell::new(Vec::new()),
        });
        let raw = new_q
            .as_com_ref::<IParamValueQueue>()
            .map(|r| r.as_ptr())
            .unwrap_or(std::ptr::null_mut());
        let i = queues.len();
        queues.push(new_q);
        if !index.is_null() {
            std::ptr::write(index, i as int32);
        }
        raw
    }
}

/// Allocate a fresh output IParameterChanges to attach to ProcessData.
pub(crate) fn new_output_changes() -> ComWrapper<OutputParameterChangesImpl> {
    ComWrapper::new(OutputParameterChangesImpl {
        queues: UnsafeCell::new(Vec::new()),
    })
}

/// Drain all points the plugin wrote during the last process() call.
/// Multiple points per paramID are flattened and returned in queue order;
/// the caller typically only needs the last value per paramID for state
/// sync, but we return them all for completeness.
pub(crate) fn drain_output(out: &ComWrapper<OutputParameterChangesImpl>) -> Vec<PendingParam> {
    let queues: Vec<_> = unsafe { std::mem::take(&mut *out.queues.get()) };
    let mut result = Vec::new();
    for q in queues {
        let id = q.id;
        let pts: Vec<(int32, ParamValue)> = unsafe { std::mem::take(&mut *q.points.get()) };
        for (off, value) in pts {
            result.push(PendingParam {
                id,
                value,
                sample_offset: off,
            });
        }
    }
    result
}
